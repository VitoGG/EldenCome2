//! enemy_dirs sheet, rule `peak_velocity_learned`: a swing's KCD2 zone is the screen-plane direction the
//! blade comes FROM (opposite of its motion, as seen through the player's camera) at its fastest.
//! - `onset`: the first frame the blade passes ONSET_SPEED_MPS gives an early verdict (first sighting).
//! - `peak`: over the whole recording, the fastest frame gives the final verdict (learned, lab).
//! Thrusts: at the peak, speed along the view axis > THRUST_RATIO x screen-plane speed.

use eldenring::cs::ChrIns;
use glam::{Vec2, Vec3};

use crate::game::Cam;
use crate::generated::{MIN_EXTENT_M, ONSET_SPEED_MPS, THRUST_RATIO, TRACKED_BONES};
use crate::kcd2::{Zone, zone_at};
use crate::skeleton::{BONES, to_world};

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Zone(i32, String),
    Thrust,
    /// No tracked bone / pose on this skeleton (fallback_no_bone).
    NoBone,
}

impl Verdict {
    pub fn name(&self) -> &str {
        match self {
            Verdict::Zone(_, n) => n,
            Verdict::Thrust => "thrust",
            Verdict::NoBone => "no_bone",
        }
    }
}

#[derive(Clone, Copy)]
struct Sample {
    t: f32,
    tip: Vec3,
    right: Vec3,
    up: Vec3,
    fwd: Vec3,
}

pub struct SwingTrack {
    pub anim: i32,
    pub elapsed: f32,
    bone: Option<usize>,
    no_bone: bool,
    samples: Vec<Sample>,
    pub onset: Option<Verdict>,
}

/// Zone for a screen-plane motion `v` (the blade comes from the opposite side).
fn zone_of(screen_v: Vec2, zones: &[Zone]) -> Verdict {
    let from = -screen_v;
    let deg = from.y.atan2(from.x).to_degrees().rem_euclid(360.0);
    zone_at(zones, deg).map_or(Verdict::Thrust, |z| Verdict::Zone(z.id, z.name.clone()))
}

fn screen(v: Vec3, s: &Sample) -> Vec2 {
    Vec2::new(v.dot(s.right), v.dot(s.up))
}

impl SwingTrack {
    pub fn new(anim: i32) -> Self {
        SwingTrack { anim, elapsed: 0.0, bone: None, no_bone: false, samples: Vec::new(), onset: None }
    }

    pub fn has_bone(&self) -> bool {
        !self.no_bone
    }

    fn extent(&self) -> f32 {
        let pts: Vec<Vec2> = self.samples.iter().map(|s| Vec2::new(s.tip.dot(s.right), s.tip.dot(s.up))).collect();
        let (lo, hi) = pts.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        if pts.is_empty() { 0.0 } else { (hi - lo).length() }
    }

    /// Record one frame; sets `onset` the first frame the blade is fast enough.
    pub fn sample(&mut self, chr: &ChrIns, cam: &Cam, zones: &[Zone], dt: f32) {
        self.elapsed += dt;
        if self.no_bone {
            return;
        }
        let mut bones = BONES.lock().unwrap();
        bones.settle(chr);
        if self.bone.is_none() {
            match bones.find(chr, TRACKED_BONES) {
                Some((i, name)) => {
                    crate::log::once(&format!("bone-{name}-{i}"), &format!("[hook:bones] tracking bone {name} (#{i})"));
                    self.bone = Some(i);
                }
                None => {
                    self.no_bone = true;
                    return;
                }
            }
        }
        let Some(mp) = bones.model_pos(chr, self.bone.unwrap()) else { return };
        drop(bones);
        let s = Sample { t: self.elapsed, tip: to_world(chr, mp), right: cam.right, up: cam.up, fwd: cam.fwd };
        let n = self.samples.len();
        if self.onset.is_none() && n >= 2 {
            let prev = self.samples[n - 2];
            let sv = screen((s.tip - prev.tip) / (s.t - prev.t).max(1e-4), &s);
            if sv.length() >= ONSET_SPEED_MPS {
                self.onset = Some(zone_of(sv, zones));
            }
        }
        self.samples.push(s);
    }

    /// Peak-velocity verdict over everything recorded, with the time of the peak.
    pub fn peak(&self, zones: &[Zone]) -> (Verdict, f32) {
        if self.no_bone || self.samples.len() < 4 {
            return (Verdict::NoBone, 0.0);
        }
        // fastest screen-plane motion over two-frame steps (the pose buffer repeats on alternate frames)
        let mut best = (0usize, Vec2::ZERO);
        for i in 2..self.samples.len() {
            let (a, b) = (&self.samples[i - 2], &self.samples[i]);
            let sv = screen((b.tip - a.tip) / (b.t - a.t).max(1e-4), b);
            if sv.length() > best.1.length() {
                best = (i, sv);
            }
        }
        let pk = &self.samples[best.0];
        if self.extent() < MIN_EXTENT_M {
            return (Verdict::Thrust, pk.t);
        }
        // thrust test over the 0.15 s around the peak
        let win: Vec<&Sample> = self.samples.iter().filter(|s| s.t >= pk.t - 0.10 && s.t <= pk.t + 0.05).collect();
        if let (Some(a), Some(b)) = (win.first(), win.last()) {
            let d = b.tip - a.tip;
            let sd = screen(d, pk).length();
            let dd = d.dot(pk.fwd).abs();
            if dd > THRUST_RATIO * sd {
                return (Verdict::Thrust, pk.t);
            }
        }
        (zone_of(best.1, zones), pk.t)
    }

    /// Compact series for the log: "t:deg/speed" of the screen-plane motion, every other sample.
    pub fn series(&self) -> String {
        self.samples
            .windows(3)
            .step_by(2)
            .map(|w| {
                let sv = screen((w[2].tip - w[0].tip) / (w[2].t - w[0].t).max(1e-4), &w[2]);
                format!("{:.2}:{:.0}/{:.1}", w[2].t, (-sv).y.atan2((-sv).x).to_degrees().rem_euclid(360.0), sv.length())
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}
