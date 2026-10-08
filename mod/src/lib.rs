//! Kingdom Come: Tarnished — Kingdom Come: Deliverance II's directional sword combat in Elden Ring.
//! Loaded by ModEngine2 (external_dlls). Reads KCD2's combat tables, HUD icons and sounds from the
//! player's own KCD2 install; ships none of them.

mod audio;
mod behavior;
mod combat;
mod dump;
mod game;
#[rustfmt::skip]
mod generated;
mod hud;
mod input;
mod kcd2;
mod lab;
mod learned;
mod log;
mod skeleton;
mod tracker;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eldenring::cs::{CSTaskGroupIndex, CSTaskImp};
use eldenring::fd4::FD4TaskData;
use eldenring::util::system::wait_for_system_init;
use fromsoftware_shared::program::Program;
use fromsoftware_shared::{FromStatic, SharedTaskImpExt};
use hudhook::Hudhook;
use hudhook::hooks::dx12::ImguiDx12Hooks;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;

const VERSION: &str = env!("CARGO_PKG_VERSION");

static COMBAT: Mutex<Option<combat::Combat>> = Mutex::new(None);

fn dll_dir(hmodule: usize) -> PathBuf {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(HMODULE(hmodule as *mut _)), &mut buf) } as usize;
    PathBuf::from(String::from_utf16_lossy(&buf[..n])).parent().map(|p| p.to_path_buf()).unwrap_or_default()
}

fn boot(hmodule: usize) {
    log::init();
    logf!("Kingdom Come: Tarnished {VERSION} loading from {}", dll_dir(hmodule).display());

    let kcd = kcd2::locate(&dll_dir(hmodule)).ok_or_else(|| "Kingdom Come: Deliverance II folder not found".to_string()).and_then(kcd2::load);
    let kcd = match kcd {
        Ok(k) => Some(Arc::new(k)),
        Err(e) => {
            logf!("[kcd2] ERROR: {e}");
            hud::HUD.lock().unwrap().banner = Some(format!("Kingdom Come: Tarnished is off: {e}"));
            None
        }
    };

    if let Err(e) = wait_for_system_init(&Program::current(), Duration::MAX) {
        logf!("[hook:boot] ERROR waiting for system init: {e:?}");
        return;
    }
    logf!("[hook:boot] game systems initialised");
    behavior::init();

    let overlay = hud::Overlay::new(kcd.clone());
    std::thread::spawn(move || {
        if let Err(e) = Hudhook::builder().with::<ImguiDx12Hooks>(overlay).with_hmodule(hudhook::windows::Win32::Foundation::HINSTANCE(hmodule as *mut _)).build().apply() {
            logf!("[hook:overlay] ERROR applying D3D12 hooks: {e:?}");
        }
    });

    let Some(kcd) = kcd else { return };
    {
        let k = kcd.clone();
        std::thread::spawn(move || {
            if let Some(b) = &k.sound_bank {
                audio::init(b);
            }
        });
    }
    learned::Learned::load();
    *COMBAT.lock().unwrap() = Some(combat::Combat::new(kcd));

    let Ok(task) = CSTaskImp::wait_for_instance(Duration::MAX) else {
        logf!("[hook:boot] ERROR: CSTask not found");
        return;
    };
    // Right after PadStep: read and withhold inputs before characters consume them.
    let h1 = task.run_recurring(
        |d: &FD4TaskData| {
            if let Some(c) = COMBAT.lock().unwrap().as_mut() {
                c.pre_input(d.delta_time.time);
            }
        },
        CSTaskGroupIndex::GameFlowStep,
    );
    // After physics: act, track swings, apply defense rules, feed the HUD.
    let h2 = task.run_recurring(
        |d: &FD4TaskData| {
            if let Some(c) = COMBAT.lock().unwrap().as_mut() {
                c.update(d.delta_time.time);
            }
        },
        CSTaskGroupIndex::ChrIns_PostPhysics,
    );
    std::mem::forget(h1);
    std::mem::forget(h2);
    logf!("[hook:boot] tasks registered (GameFlowStep, ChrIns_PostPhysics)");
}

/// # Safety
/// Called by the Windows loader only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(hmodule: usize, reason: u32, _: *mut core::ffi::c_void) -> i32 {
    if reason == 1 {
        // Never scan memory or hook from DllMain: start a thread.
        std::thread::spawn(move || boot(hmodule));
    }
    1
}

#[allow(unused_imports)]
use FromStatic as _;
