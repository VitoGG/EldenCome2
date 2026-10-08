//! Swing lab (KCT_LAB=1 only; never on for players). Fires swings on the player, tracks the player's own
//! weapon bone through each, and records the measured KCD2 zone in KCT_LAB_DIR/lab_results.jsonl
//! (tools/lab_apply.py fills swing_map's measured columns from it).
//! F8 on/off · F9 next swing_map row · F10 repeat · F11 run all swing_map rows for this class + grip ·
//! F6 explore: every candidate event for this grip · F7 dump the player's memory graph · F5 play the next
//! KCD2 clash sound.

use std::io::Write;

use eldenring::cs::PlayerIns;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11};

use crate::game::Cam;
use crate::generated::SWING_MAP;
use crate::kcd2::Kcd2Data;
use crate::tracker::SwingTrack;
use crate::{behavior, logf};

const RECORD_S: f32 = 1.4;
const GAP_S: f32 = 0.9;

/// Candidate events explored per grip ("Right" is replaced by "Both" when two-handing).
const EXPLORE: &[&str] = &[
    "W_AttackRightLight1", "W_AttackRightLight2", "W_AttackRightLight3", "W_AttackRightLight4", "W_AttackRightLight5",
    "W_AttackRightLight6", "W_AttackRightHeavy1Start", "W_AttackRightHeavy2Start", "W_AttackRightLightStep",
    "W_AttackRightLightFastStep", "W_AttackRightBackstep", "W_AttackRightLightDash", "W_AttackRightHeavyDash",
    "W_AttackRightLightStealth", "W_AttackRightLightCounter", "W_AttackRightHeavyCounter", "W_AttackRightLightSubStart",
];

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Rows,
    Explore,
}

pub struct Lab {
    on: bool,
    keys_prev: [bool; 7],
    sound_i: usize,
    req: Option<bool>, // Some(advance)
    auto: Option<Mode>,
    mode: Mode,
    row: usize,
    explore_i: usize,
    gap: f32,
    rec: Option<(String, String, SwingTrack)>, // (label, event, track)
    last: String,
    done_in_auto: usize,
    dump_req: bool,
    hand_checked: bool,
}

impl Default for Lab {
    fn default() -> Self {
        Lab {
            on: false,
            keys_prev: [false; 7],
            sound_i: 0,
            req: None,
            auto: None,
            mode: Mode::Rows,
            row: 0,
            explore_i: 0,
            gap: 0.0,
            rec: None,
            last: String::new(),
            done_in_auto: 0,
            dump_req: false,
            hand_checked: false,
        }
    }
}

fn key(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

impl Lab {
    pub fn keys(&mut self) {
        let now = [key(VK_F8.0), key(VK_F9.0), key(VK_F10.0), key(VK_F11.0), key(VK_F7.0), key(VK_F6.0), key(VK_F5.0)];
        let pr: Vec<bool> = now.iter().zip(self.keys_prev.iter()).map(|(n, p)| *n && !*p).collect();
        self.keys_prev = now;
        if pr[0] {
            self.on = !self.on;
            logf!("[lab] {}", if self.on { "on" } else { "off" });
        }
        if !self.on {
            return;
        }
        if pr[1] {
            self.mode = Mode::Rows;
            self.req = Some(true);
        }
        if pr[2] {
            self.req = Some(false);
        }
        if pr[4] {
            self.dump_req = true;
        }
        if pr[6] {
            let ids = ["clash_block", "clash_perfect", "clash_riposte"];
            let id = ids[self.sound_i % ids.len()];
            self.sound_i += 1;
            logf!("[lab] sound test {id}");
            crate::audio::play(id);
        }
        if pr[3] || pr[5] {
            let m = if pr[3] { Mode::Rows } else { Mode::Explore };
            self.mode = m;
            self.auto = Some(m);
            self.done_in_auto = 0;
            self.explore_i = 0;
            self.row = usize::MAX; // restart from the class's first row
            self.req = Some(false);
            logf!("[lab] auto run ({})", if m == Mode::Rows { "swing_map rows" } else { "explore" });
        }
    }

    fn rows_for(class: &str) -> Vec<usize> {
        SWING_MAP.iter().enumerate().filter(|(_, r)| r.weapon_class == class).map(|(i, _)| i).collect()
    }

    pub fn text(&self) -> String {
        if !self.on {
            return String::new();
        }
        format!("LAB {}| {}", if self.auto.is_some() { "AUTO " } else { "" }, self.last)
    }

    pub fn update(&mut self, p: &mut PlayerIns, cam: &Cam, kcd: &Kcd2Data, class: Option<&str>, two_handed: bool, dt: f32) {
        if !self.on {
            return;
        }
        if self.dump_req {
            self.dump_req = false;
            let chr = &p.chr_ins as *const _ as usize;
            let beh = (&*p.chr_ins.modules.behavior) as *const _ as usize;
            let hkb = behavior::hkb_character(&p.chr_ins).unwrap_or(0);
            let model = (&*p.chr_ins.chr_model_ins) as *const _ as usize;
            logf!("[dump] roots chr {chr:#x} behavior {beh:#x} hkb {hkb:#x} model {model:#x}");
            crate::dump::spawn(vec![("hkb", hkb), ("hkb_owner", hkb - 0x30), ("behavior", beh), ("model", model), ("chr", chr)], 4, 0x400, 60_000);
        }
        {
            let mut b = crate::skeleton::BONES.lock().unwrap();
            if b.settle(&p.chr_ins) && !self.hand_checked {
                if let Some((i, _)) = b.find(&p.chr_ins, &["R_Hand"]) {
                    if let Some(mp) = b.model_pos(&p.chr_ins, i) {
                        self.hand_checked = true;
                        let w = crate::skeleton::to_world(&p.chr_ins, mp);
                        let feet = crate::game::pos(&p.chr_ins);
                        logf!("[hook:bones] check: R_Hand model {mp:.2?} world {w:.2?}, player {feet:.2?}, offset {:.2?} (|{:.2}| m)", w - feet, (w - feet).length());
                    }
                }
            }
        }
        let Some(class) = class else {
            self.last = "equip a covered sword".into();
            return;
        };
        let grip = if two_handed { "2h" } else { "1h" };
        let rows = Self::rows_for(class);
        if !rows.contains(&self.row) {
            self.row = rows[0];
        }

        if let Some((label, ev, tr)) = self.rec.as_mut() {
            tr.sample(&p.chr_ins, cam, &kcd.zones, dt);
            if tr.elapsed >= RECORD_S {
                let (peak, at) = tr.peak(&kcd.zones);
                self.last = format!("{ev}: {} ({label})", peak.name());
                logf!("[lab] {class} {grip} {label} {ev}: peak={} at {:.2}s onset={:?} series {}", peak.name(), at, tr.onset.as_ref().map(|o| o.name().to_string()), tr.series());
                let line = format!(
                    "{{\"mode\":\"{}\",\"weapon_class\":\"{class}\",\"grip\":\"{grip}\",\"label\":\"{label}\",\"event\":\"{ev}\",\"peak\":\"{}\",\"peak_t\":{at:.2}}}\n",
                    if self.mode == Mode::Rows { "rows" } else { "explore" },
                    peak.name()
                );
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::log::lab_dir().join("lab_results.jsonl")) {
                    let _ = f.write_all(line.as_bytes());
                }
                self.rec = None;
                self.gap = GAP_S;
                if let Some(m) = self.auto {
                    self.done_in_auto += 1;
                    let total = if m == Mode::Rows { rows.len() } else { EXPLORE.len() };
                    if self.done_in_auto >= total {
                        self.auto = None;
                        logf!("[lab] auto run done ({total} swings)");
                    } else {
                        self.req = Some(true);
                    }
                }
            }
            return;
        }
        if self.gap > 0.0 {
            self.gap -= dt;
            return;
        }
        let Some(advance) = self.req.take() else { return };
        let (label, ev) = match self.mode {
            Mode::Rows => {
                if advance {
                    let pos = rows.iter().position(|r| *r == self.row).unwrap_or(0);
                    self.row = rows[(pos + 1) % rows.len()];
                }
                let r = &SWING_MAP[self.row];
                (format!("{}/{}", r.attack, r.kcd2_zone), if two_handed { r.event_2h } else { r.event_1h }.to_string())
            }
            Mode::Explore => {
                if advance {
                    self.explore_i = (self.explore_i + 1) % EXPLORE.len();
                }
                let base = EXPLORE[self.explore_i];
                ("explore".to_string(), if two_handed { base.replace("Right", "Both") } else { base.to_string() })
            }
        };
        let ok = behavior::fire(&p.chr_ins, &ev);
        logf!("[hook:fire_behavior_event] lab fired {ev}: {}", if ok { "accepted" } else { "rejected" });
        if ok {
            self.rec = Some((label, ev, SwingTrack::new(crate::game::anim_id(&p.chr_ins))));
        } else if let Some(m) = self.auto {
            self.done_in_auto += 1;
            let total = if m == Mode::Rows { rows.len() } else { EXPLORE.len() };
            if self.done_in_auto >= total {
                self.auto = None;
            } else {
                self.req = Some(true);
            }
        }
    }
}
