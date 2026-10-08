//! hud sheet + hooks.overlay: ImGui drawn into Elden Ring's D3D12 frame by hudhook.
//! Icons are KCD2's own textures (kcd2_sources icon_*), decoded at load from the player's copy.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use glam::Vec2;
use hudhook::imgui::{self, Context, TextureId};
use hudhook::{ImguiRenderLoop, RenderContext};

use crate::generated::{Anchor, HUD as HUD_ROWS, HudRow};
use crate::kcd2::Kcd2Data;
use crate::logf;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Incoming {
    #[default]
    None,
    /// Attack started, zone not decided yet.
    Pending,
    Zone(i32),
    /// Side unknown: any guard side blocks.
    Any,
}

#[derive(Default)]
pub struct HudState {
    pub active: bool,
    pub cursor: Vec2,
    pub cursor_zone: Option<i32>,
    pub zones: Vec<(i32, f32, f32)>,
    pub target: Option<Vec2>,
    pub incoming: Incoming,
    pub perfect_left: f32,
    pub perfect_zone: Option<i32>,
    pub riposte_left: f32,
    pub lab: Option<String>,
    pub banner: Option<String>,
}

pub static HUD: Mutex<HudState> = Mutex::new(HudState {
    active: false,
    cursor: Vec2::ZERO,
    cursor_zone: None,
    zones: Vec::new(),
    target: None,
    incoming: Incoming::None,
    perfect_left: 0.0,
    perfect_zone: None,
    riposte_left: 0.0,
    lab: None,
    banner: None,
});

pub struct Overlay {
    kcd: Option<Arc<Kcd2Data>>,
    tex: HashMap<&'static str, (TextureId, [f32; 2])>,
    logged: bool,
}

impl Overlay {
    pub fn new(kcd: Option<Arc<Kcd2Data>>) -> Self {
        Overlay { kcd, tex: HashMap::new(), logged: false }
    }
}

fn row(id: &str) -> &'static HudRow {
    HUD_ROWS.iter().find(|r| r.id == id).expect("hud row")
}

fn sector_mid(from: f32, to: f32) -> f32 {
    (from + (to - from).rem_euclid(360.0) / 2.0).rem_euclid(360.0)
}

fn dir(deg: f32) -> Vec2 {
    // screen: x right, y down; zone angles: 0 right, 90 up
    let r = deg.to_radians();
    Vec2::new(r.cos(), -r.sin())
}

impl ImguiRenderLoop for Overlay {
    fn initialize<'a>(&'a mut self, _ctx: &mut Context, rc: &'a mut dyn RenderContext) {
        if let Some(k) = &self.kcd {
            for (id, img) in &k.icons {
                match rc.load_texture(&img.px, img.w, img.h) {
                    Ok(t) => {
                        self.tex.insert(id, (t, [img.w as f32, img.h as f32]));
                    }
                    Err(e) => logf!("[hook:overlay] texture {id} failed: {e:?}"),
                }
            }
        }
        logf!("[hook:overlay] ImGui D3D12 overlay initialised, {} KCD2 icons uploaded", self.tex.len());
    }

    fn render(&mut self, ui: &mut imgui::Ui) {
        let [w, h] = ui.io().display_size;
        if !self.logged {
            self.logged = true;
            logf!("[hook:overlay] first frame {w}x{h}");
        }
        let s = HUD.lock().unwrap();
        let dl = ui.get_foreground_draw_list();
        let k = h / 1080.0;

        if let Some(b) = &s.banner {
            dl.add_text([24.0 * k, 24.0 * k], [1.0, 0.85, 0.5, 1.0], b);
        }
        if let Some(l) = s.lab.as_ref().filter(|l| !l.is_empty()) {
            dl.add_text([24.0 * k, h - 48.0 * k], [0.6, 1.0, 0.6, 1.0], l);
        }
        if !s.active {
            return;
        }
        let img = |id: &str, c: Vec2, size: f32, col: [f32; 4]| {
            if let Some((t, [iw, ih])) = self.tex.get(id) {
                let half = Vec2::new(size * iw / ih, size) * 0.5;
                dl.add_image(*t, (c - half).to_array(), (c + half).to_array()).col(col).build();
            }
        };

        // star (screen_center): sector boundaries + current zone highlight
        let star = row("star");
        let centre = Vec2::new(w * 0.5, h * 0.5);
        let r = star.size_px * k * 0.5;
        for (_, from, _) in &s.zones {
            let d = dir(*from);
            dl.add_line((centre + d * r * 0.35).to_array(), (centre + d * r).to_array(), [1.0, 1.0, 1.0, 0.35]).thickness(1.5).build();
        }
        if let Some((_, from, to)) = s.zones.iter().find(|z| Some(z.0) == s.cursor_zone) {
            let span = (to - from).rem_euclid(360.0);
            let steps = 12;
            for i in 0..steps {
                let a0 = from + span * i as f32 / steps as f32;
                let a1 = from + span * (i + 1) as f32 / steps as f32;
                let (p0, p1) = (centre + dir(a0) * r * 0.4, centre + dir(a1) * r * 0.4);
                let (q0, q1) = (centre + dir(a0) * r, centre + dir(a1) * r);
                dl.add_triangle(p0.to_array(), q0.to_array(), q1.to_array(), [1.0, 0.9, 0.6, 0.12]).filled(true).build();
                dl.add_triangle(p0.to_array(), q1.to_array(), p1.to_array(), [1.0, 0.9, 0.6, 0.12]).filled(true).build();
            }
            // cursor icon in the chosen zone
            let cur = row("cursor");
            img(cur.icon.unwrap(), centre + dir(sector_mid(*from, *to)) * r * 0.72, cur.size_px * k, [1.0, 1.0, 1.0, 0.95]);
        }

        // incoming attack on the locked target
        if let (Some(t), inc) = (s.target, s.incoming) {
            let row_i = row("incoming");
            debug_assert_eq!(row_i.anchor, Anchor::LockedTarget);
            let tc = Vec2::new(t.x * w, t.y * h);
            let red = [1.0, 0.25, 0.2, 0.95];
            let rr = 70.0 * k;
            match inc {
                Incoming::Zone(z) => {
                    if let Some((_, from, to)) = s.zones.iter().find(|q| q.0 == z) {
                        img(row_i.icon.unwrap(), tc + dir(sector_mid(*from, *to)) * rr, row_i.size_px * k, red);
                    }
                }
                Incoming::Any => {
                    for (_, from, to) in &s.zones {
                        img(row_i.icon.unwrap(), tc + dir(sector_mid(*from, *to)) * rr, row_i.size_px * k * 0.7, [1.0, 0.5, 0.3, 0.7]);
                    }
                }
                Incoming::Pending => {
                    dl.add_circle(tc.to_array(), rr * 0.5, [1.0, 0.3, 0.2, 0.6]).thickness(2.0).build();
                }
                Incoming::None => {}
            }
        }

        if s.perfect_left > 0.0 {
            let pb = row("perfect_flash");
            let at = s.perfect_zone.and_then(|z| s.zones.iter().find(|q| q.0 == z)).map_or(centre, |(_, f, t)| centre + dir(sector_mid(*f, *t)) * r * 0.72);
            img(pb.icon.unwrap(), at, pb.size_px * k, [1.0, 1.0, 1.0, (s.perfect_left / 0.4).min(1.0)]);
        }
        if s.riposte_left > 0.0 {
            let rp = row("riposte_ready");
            img(rp.icon.unwrap(), centre + Vec2::new(0.0, r * 1.25), rp.size_px * k, [1.0, 1.0, 1.0, 0.9]);
        }
    }
}
