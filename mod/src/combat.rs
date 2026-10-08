//! The combat rules (attacks, inputs, defense sheets). One `Combat` lives on the game thread; `pre_input`
//! runs right after PadStep (reads + withholds inputs), `update` runs after physics (acts, tracks, HUD).

use std::sync::Arc;

use eldenring::cs::{ChrIns, PlayerIns, UserInputKey as K};
use glam::Vec2;

use crate::game;
use crate::generated::{ATTACK_ANIM_RANGE, ATTACKS, RECORD_S, SWING_MAP};
use crate::learned::{self, LEARNED};
use crate::hud::{HUD, Incoming};
use crate::input::{self, Edge};
use crate::kcd2::{Kcd2Data, zone_at};
use crate::tracker::{SwingTrack, Verdict};
use crate::{behavior, logf};

const CURSOR_DEADZONE: f32 = 0.35;
const MOUSE_GAIN: f32 = 0.004;
const UNLOCK_HOLD_S: f32 = 0.4;

#[derive(Default)]
struct Pending {
    attack: Option<(&'static str, f32)>, // (attacks.id, seconds left in buffer)
}

pub struct Combat {
    pub kcd: Arc<Kcd2Data>,
    cursor: Vec2,
    cursor_zone: Option<i32>,
    e_attack: Edge,
    e_strong: Edge,
    e_guard: Edge,
    e_lock: Edge,
    guard_down_at: f32,
    clock: f32,
    pending: Pending,
    // enemy
    target_anim: i32,
    track: Option<SwingTrack>,
    incoming: Incoming,
    guard_withheld_for_anim: Option<i32>,
    // perfect block / riposte
    last_stamina: i32,
    last_hp: i32,
    riposte_until: f32,
    perfect_until: f32,
    perfect_zone: Option<i32>,
    // lock-on helpers
    switch_frames: u8,
    pass_unlock: bool,
    // frame state shared between pre_input and update
    active: bool,
    two_handed: bool,
    class: Option<&'static str>,
    // lab
    pub lab: Option<crate::lab::Lab>,
}

pub fn is_attack_anim(anim: i32) -> bool {
    let a = anim.rem_euclid(1_000_000);
    a >= ATTACK_ANIM_RANGE.0 && a <= ATTACK_ANIM_RANGE.1
}

impl Combat {
    pub fn new(kcd: Arc<Kcd2Data>) -> Self {
        let lab = std::env::var("KCT_LAB").ok().filter(|v| v == "1").map(|_| crate::lab::Lab::default());
        if lab.is_some() {
            logf!("[lab] swing lab enabled (F8 on/off, F9 next swing, F10 repeat)");
        }
        Combat {
            kcd,
            cursor: Vec2::new(1.0, 0.0),
            cursor_zone: None,
            e_attack: Edge::default(),
            e_strong: Edge::default(),
            e_guard: Edge::default(),
            e_lock: Edge::default(),
            guard_down_at: -10.0,
            clock: 0.0,
            pending: Pending::default(),
            target_anim: -1,
            track: None,
            incoming: Incoming::None,
            guard_withheld_for_anim: None,
            last_stamina: -1,
            last_hp: -1,
            riposte_until: -1.0,
            perfect_until: -1.0,
            perfect_zone: None,
            switch_frames: 0,
            pass_unlock: false,
            active: false,
            two_handed: false,
            class: None,
            lab,
        }
    }

    fn zone_name(&self, id: i32) -> &str {
        self.kcd.zones.iter().find(|z| z.id == id).map_or("?", |z| z.name.as_str())
    }

    /// Right after PadStep: read the controls and withhold the ones we take over.
    pub fn pre_input(&mut self, dt: f32) {
        self.clock += dt;
        let Some(p) = game::player() else {
            self.active = false;
            return;
        };
        let w = game::right_weapon(p);
        if let Some(w) = &w {
            crate::log::once(
                &format!("wep-{}-{}", w.param_id, w.two_handed),
                &format!("[hook:player] right weapon param {} wepType {} class {:?} two-handed {}", w.param_id, w.wep_type, w.class, w.two_handed),
            );
        }
        self.class = w.as_ref().and_then(|w| w.class);
        self.two_handed = w.as_ref().is_some_and(|w| w.two_handed);
        let locked = game::locked_target(p).is_some();
        self.active = locked && self.class.is_some();

        if let Some(lab) = self.lab.as_mut() {
            lab.keys();
        }

        // Lock button: tap = switch target, hold = release (inputs sheet: switch_target, release_lock).
        let lock_now = input::down(K::ResetCamera);
        let (pressed, released, _) = self.e_lock.update(lock_now, dt);
        if locked {
            if self.pass_unlock {
                self.pass_unlock = false; // this frame the game sees the press and releases lock-on
            } else {
                input::suppress(K::ResetCamera);
                input::suppress(K::ResetCameraTapped);
                if lock_now && self.e_lock.held_s >= UNLOCK_HOLD_S && self.e_lock.held_s - dt < UNLOCK_HOLD_S {
                    self.pass_unlock = true;
                    logf!("[input] hold lock: release lock-on");
                }
                if released && !pressed && self.e_lock.last_held_s < UNLOCK_HOLD_S {
                    // released before the hold threshold: a tap
                    self.switch_frames = 3;
                    logf!("[input] tap lock: switch target");
                }
            }
        }
        if self.switch_frames > 0 {
            self.switch_frames -= 1;
            input::inject_analog(K::MoveCameraRight, 1.0);
        }

        if !self.active {
            return;
        }
        // Cursor (inputs.cursor_mouse / cursor_stick)
        let mx = input::analog(K::MouseMovementX);
        let my = input::analog(K::MouseMovementY);
        let sx = input::analog(K::MoveCameraRight) - input::analog(K::MoveCameraLeft);
        let sy = input::analog(K::MoveCameraUp) - input::analog(K::MoveCameraDown);
        if Vec2::new(sx, sy).length() > 0.5 {
            self.cursor = Vec2::new(sx, sy).normalize();
        } else if mx != 0.0 || my != 0.0 {
            self.cursor += Vec2::new(mx, -my) * MOUSE_GAIN;
            if self.cursor.length() > 1.0 {
                self.cursor = self.cursor.normalize();
            }
        }
        if self.switch_frames == 0 {
            for k in [K::MouseMovementX, K::MouseMovementY, K::MoveCameraUp, K::MoveCameraDown, K::MoveCameraLeft, K::MoveCameraRight] {
                input::suppress(k);
            }
        }
        if self.cursor.length() >= CURSOR_DEADZONE {
            let deg = self.cursor.y.atan2(self.cursor.x).to_degrees();
            self.cursor_zone = zone_at(&self.kcd.zones, deg).map(|z| z.id);
        }

        // Attacks (attacks sheet): withheld from the game, fired by us in update().
        let (a_press, _, _) = self.e_attack.update(input::down(K::Attack), dt);
        let (s_press, _, _) = self.e_strong.update(input::down(K::StrongAttack), dt);
        input::suppress(K::Attack);
        input::suppress(K::StrongAttack);
        for (pressed, input_id) in [(a_press, "attack_slash"), (s_press, "attack_stab")] {
            if pressed {
                if let Some(a) = ATTACKS.iter().find(|a| a.input == input_id) {
                    self.pending.attack = Some((a.id, a.buffer_s));
                }
            }
        }

        // Guard (defense.block_wrong_side): withheld while a known wrong-side attack is coming.
        let (g_press, _, g_held) = self.e_guard.update(input::down(K::Guard), dt);
        if g_press {
            self.guard_down_at = self.clock;
        }
        if g_held {
            if let Incoming::Zone(z) = self.incoming {
                if Some(z) != self.cursor_zone && self.guard_withheld_for_anim != Some(self.target_anim) {
                    self.guard_withheld_for_anim = Some(self.target_anim);
                    logf!("[defense] wrong side: guarding {:?}, attack from {}: guard opens", self.cursor_zone.map(|c| self.zone_name(c).to_string()), self.zone_name(z));
                }
            }
        }
        if self.guard_withheld_for_anim.is_some() {
            if self.guard_withheld_for_anim == Some(self.target_anim) {
                input::suppress(K::Guard);
            } else {
                self.guard_withheld_for_anim = None;
            }
        }
    }

    /// End the current enemy track: learn its peak-velocity zone (enemy_dirs.learned_table).
    fn finish_track(&mut self, behavior_name: &str) {
        let Some(tr) = self.track.take() else { return };
        if tr.elapsed < 0.3 {
            return; // interrupted before it swung
        }
        let (v, at) = tr.peak(&self.kcd.zones);
        logf!("[track] {behavior_name} anim {} peak {} at {:.2}s (onset said {:?}) series {}", tr.anim, v.name(), at, tr.onset.as_ref().map(|o| o.name().to_string()), tr.series());
        if v != Verdict::NoBone {
            let mut l = LEARNED.lock().unwrap();
            l.set(learned::key(behavior_name, tr.anim), v.name());
            l.save_if_dirty();
        }
    }

    fn swing_event(&self, attack: &str) -> Option<&'static str> {
        let class = self.class?;
        let zone = if attack == "stab" {
            "-".to_string()
        } else {
            self.zone_name(self.cursor_zone?).to_string()
        };
        let row = SWING_MAP.iter().find(|r| r.weapon_class == class && r.attack == attack && r.kcd2_zone == zone)?;
        Some(if self.two_handed { row.event_2h } else { row.event_1h })
    }

    fn can_attack(p: &PlayerIns) -> bool {
        let ar = &p.chr_ins.modules.action_request;
        ar.possible_action_cancels.r1() && p.chr_ins.modules.data.stamina > 0
    }

    /// After physics: act on the inputs, track the target's swing, apply the defense rules, feed the HUD.
    pub fn update(&mut self, dt: f32) {
        let Some(p) = game::player() else { return };
        let cam = game::camera();
        if let (Some(lab), Some(cam)) = (self.lab.as_mut(), cam.as_ref()) {
            lab.update(p, cam, &self.kcd, self.class, self.two_handed, dt);
        }
        let target: Option<&mut ChrIns> = game::locked_target(p);
        let mut hud_target = None;

        if let (Some(t), Some(cam)) = (target, cam.as_ref()) {
            let tp = game::pos(t) + glam::Vec3::Y * 1.3;
            // hooks.camera: the locked target is always in front; calibrate the forward sign once.
            let z = (tp - cam.pos).dot(cam.fwd);
            if z < -1.0 {
                let s = -game::FWD_SIGN.load(std::sync::atomic::Ordering::Relaxed);
                game::FWD_SIGN.store(s, std::sync::atomic::Ordering::Relaxed);
                logf!("[hook:camera] forward sign flipped to {s}");
            } else if z > 1.0 {
                crate::log::once("cam-ok", &format!("[hook:camera] target {:.1} m in front, fov {:.2}", z, cam.fov_y));
            }
            hud_target = cam.project(tp);

            // hooks.target + enemy_dirs (peak_velocity_learned): a new attack animation starts a track.
            // A learned zone shows at once; otherwise the strike onset decides, and the peak is learned.
            let anim = game::anim_id(t);
            let behavior_name = t.modules.data.character_behavior_name.to_string().unwrap_or_default();
            if anim != self.target_anim {
                self.finish_track(&behavior_name);
                crate::log::once(&format!("tanim-{behavior_name}-{anim}"), &format!("[hook:target] {behavior_name} anim {anim} ({})", if is_attack_anim(anim) { "attack" } else { "other" }));
                self.target_anim = anim;
                self.track = if is_attack_anim(anim) { Some(SwingTrack::new(anim)) } else { None };
                self.incoming = Incoming::None;
                if self.track.is_some() {
                    let known = LEARNED.lock().unwrap().get(&learned::key(&behavior_name, anim)).map(str::to_string);
                    self.incoming = match known.as_deref() {
                        Some(z) => self.kcd.zones.iter().find(|q| q.name == z).map_or(Incoming::Any, |q| Incoming::Zone(q.id)),
                        None => Incoming::Pending,
                    };
                }
            }
            if let Some(tr) = self.track.as_mut() {
                if tr.elapsed < RECORD_S {
                    tr.sample(t, cam, &self.kcd.zones, dt);
                    if !tr.has_bone() {
                        self.incoming = Incoming::Any;
                    } else if self.incoming == Incoming::Pending {
                        if let Some(v) = &tr.onset {
                            self.incoming = match v {
                                Verdict::Zone(id, _) => Incoming::Zone(*id),
                                _ => Incoming::Any,
                            };
                        }
                    }
                } else {
                    self.finish_track(&behavior_name);
                }
            }

            // defense.perfect_block: a hit landed on the guard (stamina dropped, HP did not).
            let st = game::stamina(&p.chr_ins);
            let hp = p.chr_ins.modules.data.hp;
            let guarding = self.e_guard.held_s > 0.0 && self.guard_withheld_for_anim.is_none();
            if self.last_stamina >= 0 && guarding && is_attack_anim(self.target_anim) && st < self.last_stamina - 2 && hp >= self.last_hp {
                let since_guard = self.clock - self.guard_down_at;
                let zone_ok = match self.incoming {
                    Incoming::Zone(z) => Some(z) == self.cursor_zone,
                    _ => true,
                };
                logf!("[hook:guard_hit_detect] guard hit: stamina {}->{}, guard held {:.3}s, zone ok {}", self.last_stamina, st, since_guard, zone_ok);
                if zone_ok && since_guard <= self.kcd.perfect_window_s {
                    let before = self.last_stamina;
                    game::set_stamina(&mut p.chr_ins, before);
                    t.modules.toughness.toughness = 0.0;
                    self.riposte_until = self.clock + self.kcd.riposte_window_s;
                    self.perfect_until = self.clock + 0.4;
                    self.perfect_zone = self.cursor_zone;
                    crate::audio::play("clash_perfect");
                    logf!("[defense] PERFECT BLOCK: stamina restored to {}, target poise -> 0, riposte window {:.2}s", before, self.kcd.riposte_window_s);
                } else {
                    crate::audio::play("clash_block");
                }
            }
            self.last_stamina = game::stamina(&p.chr_ins);
            self.last_hp = hp;
        } else {
            self.track = None;
            self.incoming = Incoming::None;
            self.target_anim = -1;
            self.last_stamina = -1;
        }

        // Attacks: fire the buffered one as soon as the game would accept an R1.
        if let Some((atk, left)) = self.pending.attack.take() {
            if self.active && Self::can_attack(p) {
                let riposte = self.clock < self.riposte_until && atk == "slash";
                let event = if riposte {
                    Some(if self.two_handed { "W_AttackBothHeavyCounter" } else { "W_AttackRightHeavyCounter" })
                } else {
                    self.swing_event(atk)
                };
                if let Some(ev) = event {
                    let ok = behavior::fire(&p.chr_ins, ev);
                    logf!(
                        "[attack] {} zone {:?} -> {} ({})",
                        if riposte { "RIPOSTE" } else { atk },
                        self.cursor_zone.map(|z| self.zone_name(z).to_string()),
                        ev,
                        if ok { "ok" } else { "rejected" }
                    );
                    if ok {
                        if riposte {
                            self.riposte_until = -1.0;
                            crate::audio::play("clash_riposte");
                        }
                        self.drain_attack_stamina(p, atk);
                    }
                } else {
                    logf!("[attack] {atk}: no swing for class {:?} zone {:?}", self.class, self.cursor_zone);
                }
            } else if left - dt > 0.0 && self.active {
                self.pending.attack = Some((atk, left - dt));
            }
        }

        let mut h = HUD.lock().unwrap();
        h.active = self.active;
        h.cursor = self.cursor;
        h.cursor_zone = self.cursor_zone;
        h.zones = self.kcd.zones.iter().map(|z| (z.id, z.from, z.to)).collect();
        h.target = hud_target;
        h.incoming = self.incoming;
        h.perfect_left = (self.perfect_until - self.clock).max(0.0);
        h.perfect_zone = self.perfect_zone;
        h.riposte_left = (self.riposte_until - self.clock).max(0.0);
        h.lab = self.lab.as_ref().map(|l| l.text());
    }

    /// attacks.extra_stamina: (relative_stamina_cost - 1) * BaseAttackStaminaCost * stamina_scale on top of Elden Ring's cost.
    fn drain_attack_stamina(&self, p: &mut PlayerIns, atk: &str) {
        let Some(row) = ATTACKS.iter().find(|a| a.id == atk) else { return };
        let rel = self.kcd.relative_stamina.get(row.kcd2_attack_type).copied().unwrap_or(1.0);
        let extra = ((rel - 1.0) * self.kcd.base_attack_stamina * row.stamina_scale).round() as i32;
        if extra > 0 {
            let s = game::stamina(&p.chr_ins);
            game::set_stamina(&mut p.chr_ins, s - extra);
        }
    }
}
