//! hooks.player / hooks.target / hooks.camera / hooks.player_stamina: reading Elden Ring through fromsoftware-rs.

use eldenring::cs::{CSCamera, ChrAsmHand, ChrIns, EquipParamWeapon, PlayerIns, SoloParamRepository, WorldChrMan};
use fromsoftware_shared::FromStatic;
use std::sync::atomic::{AtomicI32, Ordering};

use glam::{Vec2, Vec3};

use crate::generated::WEAPON_CLASSES;

pub fn world() -> Option<&'static mut WorldChrMan> {
    unsafe { WorldChrMan::instance_mut() }.ok()
}

pub fn player() -> Option<&'static mut PlayerIns> {
    world()?.main_player.as_mut().map(|p| &mut **p)
}

pub fn pos(chr: &ChrIns) -> Vec3 {
    let p = &chr.modules.physics.position;
    Vec3::new(p.0, p.1, p.2)
}

/// Current animation id of a character (the newest entry of its TimeAct queue).
pub fn anim_id(chr: &ChrIns) -> i32 {
    let ta = &chr.modules.time_act;
    let i = (ta.read_idx as usize) % ta.anim_queue.len();
    ta.anim_queue[i].anim_id
}

/// Seconds into the current animation.
pub fn anim_time(chr: &ChrIns) -> f32 {
    let ta = &chr.modules.time_act;
    let i = (ta.read_idx as usize) % ta.anim_queue.len();
    ta.anim_queue[i].play_time
}

pub struct RightWeapon {
    pub param_id: i32,
    pub wep_type: u16,
    pub class: Option<&'static str>,
    pub two_handed: bool,
}

pub fn right_weapon(p: &PlayerIns) -> Option<RightWeapon> {
    let asm = &p.chr_asm;
    let slot = asm.equipment.active_weapon_slot(ChrAsmHand::Right) as usize;
    let id = *asm.equipment_param_ids.get(slot)?;
    if id <= 0 {
        return None;
    }
    let repo = unsafe { SoloParamRepository::instance() }.ok()?;
    let row = repo.get::<EquipParamWeapon>(((id / 100) * 100) as u32)?;
    let wep_type = row.wep_type();
    let class = WEAPON_CLASSES.iter().find(|c| c.er_wep_type as u16 == wep_type).map(|c| c.id);
    Some(RightWeapon { param_id: id, wep_type, class, two_handed: asm.equipment.is_two_handing() })
}

pub fn locked_target(p: &PlayerIns) -> Option<&'static mut ChrIns> {
    if !p.chr_ins.is_locked_on || p.locked_on_enemy.is_empty() {
        return None;
    }
    let h = p.locked_on_enemy;
    world()?.chr_ins_by_handle_mut(&h)
}

/// +1 or -1: whether the camera matrix's third row points where the camera looks. Calibrated from the
/// locked target (always in front) and logged as [hook:camera].
pub static FWD_SIGN: AtomicI32 = AtomicI32::new(1);

pub struct Cam {
    pub pos: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub fwd: Vec3,
    pub fov_y: f32,
    pub aspect: f32,
}

pub fn camera() -> Option<Cam> {
    let c = unsafe { CSCamera::instance() }.ok()?;
    let cam: &eldenring::cs::CSCam = &c.pers_cam_1;
    let m = &cam.matrix;
    let v = |r: &fromsoftware_shared::F32Vector4| Vec3::new(r.0, r.1, r.2);
    let s = FWD_SIGN.load(Ordering::Relaxed) as f32;
    Some(Cam { right: v(&m.0), up: v(&m.1), fwd: v(&m.2) * s, pos: v(&m.3), fov_y: cam.fov, aspect: cam.aspect_ratio })
}

impl Cam {
    /// Screen position in 0..1 (x right, y down), or None when behind the camera.
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        let d = p - self.pos;
        let z = d.dot(self.fwd);
        if z <= 0.05 {
            return None;
        }
        let t = (self.fov_y * 0.5).tan();
        let x = d.dot(self.right) / (z * t * self.aspect);
        let y = d.dot(self.up) / (z * t);
        Some(Vec2::new(0.5 + x * 0.5, 0.5 - y * 0.5))
    }

    /// Direction in the screen plane (x right, y up), unnormalised.
    pub fn screen_dir(&self, world_delta: Vec3) -> Vec2 {
        Vec2::new(world_delta.dot(self.right), world_delta.dot(self.up))
    }
}

pub fn stamina(chr: &ChrIns) -> i32 {
    chr.modules.data.stamina
}

pub fn set_stamina(chr: &mut ChrIns, v: i32) {
    let d = &mut chr.modules.data;
    d.stamina = v.clamp(0, d.max_stamina);
}
