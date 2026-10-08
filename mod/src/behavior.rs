//! hooks.fire_behavior_event: start a named Havok behavior event (the same thing Elden Ring's HKS
//! `ExecEvent("W_...")` does) on a character. Address and hkbCharacter path from The Grand Archives
//! CT (PlayAnimation_code.cea). Call only from the game thread (our CSTask).

use std::sync::OnceLock;

use eldenring::cs::ChrIns;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::core::PCWSTR;

use crate::logf;

type FireEventFn = unsafe extern "C" fn(hkb_character: usize, name: *const u16) -> u32;

static FIRE: OnceLock<Option<FireEventFn>> = OnceLock::new();

/// Bytes of the game's main module image.
pub fn module_image() -> Option<&'static [u8]> {
    unsafe {
        let base = GetModuleHandleW(PCWSTR::null()).ok()?.0 as *const u8;
        let e_lfanew = *(base.add(0x3c) as *const u32) as usize;
        let size_of_image = *(base.add(e_lfanew + 0x18 + 0x38) as *const u32) as usize;
        Some(std::slice::from_raw_parts(base, size_of_image))
    }
}

/// Find a unique byte pattern ("74 ?? 48 ...") in the main module's executable sections.
pub fn aob_unique(pattern: &str) -> Option<usize> {
    let pat: Vec<Option<u8>> = pattern
        .split_whitespace()
        .map(|t| if t.starts_with('?') { None } else { u8::from_str_radix(t, 16).ok() })
        .collect();
    let img = module_image()?;
    let base = img.as_ptr() as usize;
    unsafe {
        let e_lfanew = *(img.as_ptr().add(0x3c) as *const u32) as usize;
        let nsec = *(img.as_ptr().add(e_lfanew + 6) as *const u16) as usize;
        let opt_size = *(img.as_ptr().add(e_lfanew + 20) as *const u16) as usize;
        let sec0 = e_lfanew + 24 + opt_size;
        let mut hits = Vec::new();
        for i in 0..nsec {
            let s = sec0 + i * 40;
            let vsize = *(img.as_ptr().add(s + 8) as *const u32) as usize;
            let va = *(img.as_ptr().add(s + 12) as *const u32) as usize;
            let chars = *(img.as_ptr().add(s + 36) as *const u32);
            if chars & 0x2000_0000 == 0 {
                continue; // not executable
            }
            let sec = &img[va..(va + vsize).min(img.len())];
            'outer: for off in 0..sec.len().saturating_sub(pat.len()) {
                for (k, b) in pat.iter().enumerate() {
                    if let Some(b) = b {
                        if sec[off + k] != *b {
                            continue 'outer;
                        }
                    }
                }
                hits.push(base + va + off);
                if hits.len() > 1 {
                    break;
                }
            }
        }
        match hits.as_slice() {
            [one] => Some(*one),
            [] => None,
            _ => {
                logf!("[aob] pattern not unique: {pattern}");
                None
            }
        }
    }
}

pub fn init() -> bool {
    let f = FIRE.get_or_init(|| {
        let hit = aob_unique("74 ?? 48 85 d2 74 ?? 48 8d 4c 24 50")?;
        let addr = hit - 0xD;
        let base = module_image()?.as_ptr() as usize;
        logf!("[hook:fire_behavior_event] found at eldenring.exe+{:#x}", addr - base);
        Some(unsafe { std::mem::transmute::<usize, FireEventFn>(addr) })
    });
    if f.is_none() {
        logf!("[hook:fire_behavior_event] ERROR: pattern not found");
    }
    f.is_some()
}

/// The character's hkbCharacter (*(behavior_module + 0x10) + 0x30).
pub fn hkb_character(chr: &ChrIns) -> Option<usize> {
    unsafe {
        let behavior = (&*chr.modules.behavior) as *const _ as usize;
        let p = *((behavior + 0x10) as *const usize);
        if p == 0 { None } else { Some(p + 0x30) }
    }
}

/// Fire a behavior event by name. Returns false when the game rejects it (0xFFFFFFFF).
pub fn fire(chr: &ChrIns, event: &str) -> bool {
    let Some(Some(f)) = FIRE.get() else { return false };
    let Some(hkb) = hkb_character(chr) else { return false };
    let wide: Vec<u16> = event.encode_utf16().chain(std::iter::once(0)).collect();
    let r = unsafe { f(hkb, wide.as_ptr()) };
    if r == 0xFFFF_FFFF {
        logf!("[behavior] event {event} rejected");
        false
    } else {
        true
    }
}
