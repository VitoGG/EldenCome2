//! hooks.bones: world positions of named bones on any character.
//!
//! Layout found in a lab memory dump of Elden Ring 2.7.1.0 (tools/analyze_dump.py, MODLOG):
//! - hkaSkeleton* at *(CSChrModelIns + 0x158). This Havok build's hkReferencedObject is 0x18 bytes, so:
//!   +0x18 name, +0x20 hkArray<i16> parentIndices, +0x30 hkArray<hkaBone{name, lock}> (stride 0x10),
//!   +0x40 hkArray<hkQsTransform> referencePose (static bind pose).
//! - live pose candidates: *(ChrIns + 0x398) + 0x50 / + 0x60 (double buffer), arrays of hkQsTransform
//!   (stride 0x30). `settle` keeps the first candidate that changes between frames and decides whether
//!   it is local (parent-relative) or model space.

use std::sync::Mutex;

use eldenring::cs::ChrIns;
use glam::{Quat, Vec3};
use windows::Win32::System::Memory::{MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery};

use crate::logf;

pub fn readable(addr: usize, len: usize) -> bool {
    if addr < 0x10000 || addr > 0x7FFF_FFFF_FFFF {
        return false;
    }
    thread_local! {
        static PAGES: std::cell::RefCell<std::collections::HashMap<usize, bool>> = std::cell::RefCell::new(std::collections::HashMap::new());
    }
    let (p0, p1) = (addr >> 12, (addr + len.max(1) - 1) >> 12);
    if p1 - p0 <= 1 {
        let hit = PAGES.with(|m| {
            let m = m.borrow();
            match (m.get(&p0), m.get(&p1)) {
                (Some(a), Some(b)) => Some(*a && *b),
                _ => None,
            }
        });
        if let Some(h) = hit {
            return h;
        }
        let r = readable_query(addr, len);
        if r {
            PAGES.with(|m| {
                let mut m = m.borrow_mut();
                if m.len() > 200_000 {
                    m.clear();
                }
                m.insert(p0, true);
                m.insert(p1, true);
            });
        }
        return r;
    }
    readable_query(addr, len)
}

fn readable_query(addr: usize, len: usize) -> bool {
    let mut mbi = MEMORY_BASIC_INFORMATION::default();
    let n = unsafe { VirtualQuery(Some(addr as *const _), &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
    if n == 0 || mbi.State != MEM_COMMIT || (mbi.Protect.0 & (PAGE_NOACCESS.0 | PAGE_GUARD.0)) != 0 || mbi.Protect.0 == 0 {
        return false;
    }
    let end = mbi.BaseAddress as usize + mbi.RegionSize;
    addr + len <= end
}

fn rd<T: Copy>(addr: usize) -> Option<T> {
    if readable(addr, std::mem::size_of::<T>()) { Some(unsafe { *(addr as *const T) }) } else { None }
}

fn cstr(addr: usize) -> Option<String> {
    let addr = addr & !1; // hkStringPtr keeps an ownership flag in bit 0
    let mut s = String::new();
    for i in 0..64 {
        let b: u8 = rd(addr + i)?;
        if b == 0 {
            break;
        }
        if !(0x21..0x7f).contains(&b) {
            return None;
        }
        s.push(b as char);
    }
    if s.is_empty() { None } else { Some(s) }
}

const SKELETON_IN_MODEL: usize = 0x158;
const SK_PARENTS: usize = 0x20;
const SK_BONES: usize = 0x30;
const POSE_ROOT_IN_CHR: usize = 0x398;
const POSE_CANDIDATES: [usize; 2] = [0x50, 0x60];

pub struct Skeleton {
    pub addr: usize,
    pub names: Vec<String>,
    pub parents: Vec<i16>,
}

fn read_skeleton(sk: usize) -> Option<Skeleton> {
    let parents: usize = rd(sk + SK_PARENTS)?;
    let np: i32 = rd(sk + SK_PARENTS + 8)?;
    let bones: usize = rd(sk + SK_BONES)?;
    let n: i32 = rd(sk + SK_BONES + 8)?;
    if !(8..=2000).contains(&n) || np != n {
        return None;
    }
    let mut names = Vec::with_capacity(n as usize);
    let mut par = Vec::with_capacity(n as usize);
    for i in 0..n as usize {
        names.push(cstr(rd::<usize>(bones + i * 0x10)?).unwrap_or_default());
        par.push(rd::<i16>(parents + i * 2)?);
    }
    if par[0] != -1 {
        return None;
    }
    Some(Skeleton { addr: sk, names, parents: par })
}

fn chr_addr(chr: &ChrIns) -> usize {
    chr as *const ChrIns as usize
}

fn skeleton_ptr(chr: &ChrIns) -> Option<usize> {
    let model = (&*chr.chr_model_ins) as *const _ as usize;
    rd::<usize>(model + SKELETON_IN_MODEL)
}

pub struct Bones {
    cache: Vec<Skeleton>,
    /// Chosen pose buffer offset (from *(ChrIns+0x398)) and whether it is model space.
    pose: Option<(usize, bool)>,
    snapshots: Vec<(usize, Vec<u32>)>,
    logged_fail: bool,
}

pub static BONES: Mutex<Bones> = Mutex::new(Bones { cache: Vec::new(), pose: None, snapshots: Vec::new(), logged_fail: false });

impl Bones {
    fn skeleton(&mut self, chr: &ChrIns) -> Option<&Skeleton> {
        let sk = skeleton_ptr(chr)?;
        if let Some(i) = self.cache.iter().position(|s| s.addr == sk) {
            return self.cache.get(i);
        }
        let Some(s) = read_skeleton(sk) else {
            if !self.logged_fail {
                self.logged_fail = true;
                let words: Vec<String> = (0..10).map(|i| rd::<u64>(sk + i * 8).map_or("??".into(), |v| format!("{v:#x}"))).collect();
                logf!("[hook:bones] no skeleton at *(model+{SKELETON_IN_MODEL:#x}) = {sk:#x}: [{}]", words.join(" "));
                if let (Some(b), Some(n)) = (rd::<usize>(sk + SK_BONES), rd::<i32>(sk + SK_BONES + 8)) {
                    let names: Vec<String> = (0..n.clamp(0, 6) as usize).map(|i| rd::<usize>(b + i * 0x10).and_then(cstr).unwrap_or("?".into())).collect();
                    let par: Vec<String> = rd::<usize>(sk + SK_PARENTS).map(|p| (0..6).map(|i| rd::<i16>(p + i * 2).map_or("?".into(), |v| v.to_string())).collect()).unwrap_or_default();
                    logf!("[hook:bones]   bones n={n} first {names:?}, parents {par:?}");
                }
            }
            return None;
        };
        crate::log::once(
            &format!("skel-{}", s.names.len()),
            &format!("[hook:bones] skeleton {} bones, first [{}]", s.names.len(), s.names.iter().take(5).cloned().collect::<Vec<_>>().join(", ")),
        );
        if self.cache.len() > 128 {
            self.cache.clear();
        }
        self.cache.push(s);
        self.cache.last()
    }

    fn pose_base(chr: &ChrIns) -> Option<usize> {
        rd::<usize>(chr_addr(chr) + POSE_ROOT_IN_CHR)
    }

    /// Call once per frame with an animated character until the live pose buffer is identified.
    pub fn settle(&mut self, chr: &ChrIns) -> bool {
        if self.pose.is_some() {
            return true;
        }
        let Some(n) = self.skeleton(chr).map(|s| s.names.len()) else { return false };
        let Some(base) = Self::pose_base(chr) else { return false };
        let mut now = Vec::new();
        for off in POSE_CANDIDATES {
            let Some(arr) = rd::<usize>(base + off) else { continue };
            if !readable(arr, n * 0x30) {
                continue;
            }
            now.push((off, (0..n.min(64) * 12).map(|k| unsafe { *((arr + k * 4) as *const u32) }).collect::<Vec<u32>>()));
        }
        for (off, snap) in &now {
            if let Some((_, prev)) = self.snapshots.iter().find(|s| s.0 == *off) {
                if prev != snap {
                    let arr = rd::<usize>(base + off).unwrap_or(0);
                    let short = (1..n)
                        .filter(|j| unsafe { Vec3::from_slice(std::slice::from_raw_parts((arr + j * 0x30) as *const f32, 3)) }.length() < 0.6)
                        .count();
                    let model_space = short < n / 2;
                    self.pose = Some((*off, model_space));
                    logf!(
                        "[hook:bones] live pose = *(ChrIns+{POSE_ROOT_IN_CHR:#x})+{off:#x} ({}), {short} of {n} bones short",
                        if model_space { "model space" } else { "local" }
                    );
                    return true;
                }
            }
        }
        self.snapshots = now;
        false
    }

    /// Index of the first of `names` present on this character's skeleton.
    pub fn find(&mut self, chr: &ChrIns, names: &[&str]) -> Option<(usize, String)> {
        let s = self.skeleton(chr)?;
        names.iter().find_map(|want| s.names.iter().position(|b| b == want).map(|i| (i, want.to_string())))
    }

    /// Model-space position of bone `idx`.
    pub fn model_pos(&mut self, chr: &ChrIns, idx: usize) -> Option<Vec3> {
        let (off, model_space) = self.pose?;
        let base = Self::pose_base(chr)?;
        let s = self.skeleton(chr)?;
        let n = s.names.len();
        let parents = s.parents.clone();
        let arr = rd::<usize>(base + off)?;
        if idx >= n || !readable(arr, n * 0x30) {
            return None;
        }
        let tr = |i: usize| -> (Vec3, Quat) {
            let f = unsafe { std::slice::from_raw_parts((arr + i * 0x30) as *const f32, 8) };
            (Vec3::new(f[0], f[1], f[2]), Quat::from_xyzw(f[4], f[5], f[6], f[7]))
        };
        if model_space {
            return Some(tr(idx).0);
        }
        let mut chain = vec![idx];
        let mut c = idx;
        while parents[c] >= 0 {
            c = parents[c] as usize;
            chain.push(c);
            if chain.len() > 256 {
                return None;
            }
        }
        let (mut pos, mut rot) = (Vec3::ZERO, Quat::IDENTITY);
        for &i in chain.iter().rev() {
            let (t, q) = tr(i);
            pos += rot * t;
            rot = rot * q;
        }
        Some(pos)
    }
}

/// World position of a model-space point on a character (ChrCtrl.model_matrix, rows = right, up, forward, position).
pub fn to_world(chr: &ChrIns, p: Vec3) -> Vec3 {
    let m = &chr.chr_ctrl.model_matrix;
    let r = |v: &fromsoftware_shared::F32Vector4| Vec3::new(v.0, v.1, v.2);
    r(&m.0) * p.x + r(&m.1) * p.y + r(&m.2) * p.z + r(&m.3)
}
