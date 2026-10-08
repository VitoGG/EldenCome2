//! Reads Kingdom Come: Deliverance II's own data from the player's install (kcd2_sources sheet).
//! Nothing read here is shipped with the mod.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::generated::{KCD2_SOURCES, Kcd2Source, SourceKind, ZONES};
use crate::logf;

#[derive(Debug, Clone)]
pub struct Zone {
    pub id: i32,
    pub name: String,
    pub label: &'static str,
    /// Sector in screen degrees, 0 = right, 90 = up, counter-clockwise from `from` to `to`.
    pub from: f32,
    pub to: f32,
}

impl Zone {
    pub fn contains(&self, deg: f32) -> bool {
        let d = deg.rem_euclid(360.0);
        let (a, b) = (self.from.rem_euclid(360.0), self.to.rem_euclid(360.0));
        if a <= b { d >= a && d < b } else { d >= a || d < b }
    }
    /// Centre of the sector, degrees.
    pub fn centre(&self) -> f32 {
        let span = (self.to - self.from).rem_euclid(360.0);
        (self.from + span / 2.0).rem_euclid(360.0)
    }
}

pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

pub struct Kcd2Data {
    pub root: PathBuf,
    pub zones: Vec<Zone>,
    pub relative_stamina: HashMap<String, f32>,
    pub base_attack_stamina: f32,
    pub perfect_window_s: f32,
    pub riposte_window_s: f32,
    pub icons: HashMap<&'static str, Rgba>,
    /// Raw FMOD bank bytes, decoded by the audio module (optional source).
    pub sound_bank: Option<Vec<u8>>,
}

/// Where the player's KCD2 is: Melty passes it in KCT_KCD2_DIR and in kctarnished.env next to the DLL.
pub fn locate(dll_dir: &Path) -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("KCT_KCD2_DIR") {
        let p = PathBuf::from(v);
        if p.join("Data").is_dir() {
            logf!("[kcd2] folder from KCT_KCD2_DIR: {}", p.display());
            return Some(p);
        }
    }
    let env = dll_dir.join("kctarnished.env");
    if let Ok(t) = std::fs::read_to_string(&env) {
        for line in t.lines() {
            if let Some(v) = line.trim().strip_prefix("KCT_KCD2_DIR=") {
                let p = PathBuf::from(v.trim().trim_matches('"'));
                if p.join("Data").is_dir() {
                    logf!("[kcd2] folder from {}: {}", env.display(), p.display());
                    return Some(p);
                }
            }
        }
    }
    logf!("[kcd2] ERROR: Kingdom Come: Deliverance II folder not found (KCT_KCD2_DIR / kctarnished.env)");
    None
}

fn read_entry(root: &Path, src: &Kcd2Source) -> Result<Vec<u8>, String> {
    let pak = root.join("Data").join(src.pak);
    let f = File::open(&pak).map_err(|e| format!("{}: {e}", pak.display()))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("{}: {e}", pak.display()))?;
    let mut e = z.by_name(src.entry).map_err(|e| format!("{} in {}: {e}", src.entry, src.pak))?;
    let mut v = Vec::with_capacity(e.size() as usize);
    e.read_to_end(&mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

fn xml_rows(bytes: &[u8], element: &str) -> Result<Vec<HashMap<String, String>>, String> {
    let text = String::from_utf8_lossy(bytes);
    let doc = roxmltree::Document::parse(&text).map_err(|e| e.to_string())?;
    Ok(doc
        .descendants()
        .filter(|n| n.has_tag_name(element))
        .map(|n| n.attributes().map(|a| (a.name().to_string(), a.value().to_string())).collect())
        .collect())
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.is_empty() { 0.0 } else if v.len() % 2 == 1 { v[v.len() / 2] } else { (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0 }
}

fn decode_dds(bytes: &[u8]) -> Result<Rgba, String> {
    if bytes.len() < 148 || &bytes[0..4] != b"DDS " {
        return Err("not a DDS".into());
    }
    let h = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    let w = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let fourcc = &bytes[84..88];
    let (data, dxgi) = if fourcc == b"DX10" {
        (&bytes[148..], u32::from_le_bytes(bytes[128..132].try_into().unwrap()))
    } else {
        (&bytes[128..], 0)
    };
    let mut out = vec![0u32; (w * h) as usize];
    match dxgi {
        98 | 99 => texture2ddecoder::decode_bc7(data, w as usize, h as usize, &mut out)?,
        71 | 72 => texture2ddecoder::decode_bc1(data, w as usize, h as usize, &mut out)?,
        77 | 78 => texture2ddecoder::decode_bc3(data, w as usize, h as usize, &mut out)?,
        _ => return Err(format!("unsupported DDS format dxgi={dxgi} fourcc={:?}", fourcc)),
    }
    // texture2ddecoder writes 0xAARRGGBB; ImGui wants RGBA bytes.
    let mut px = Vec::with_capacity(out.len() * 4);
    for p in out {
        px.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
    }
    Ok(Rgba { w, h, px })
}

pub fn load(root: PathBuf) -> Result<Kcd2Data, String> {
    let mut d = Kcd2Data {
        root: root.clone(),
        zones: Vec::new(),
        relative_stamina: HashMap::new(),
        base_attack_stamina: 0.0,
        perfect_window_s: 0.0,
        riposte_window_s: 0.0,
        icons: HashMap::new(),
        sound_bank: None,
    };
    for src in KCD2_SOURCES {
        let bytes = match read_entry(&root, src) {
            Ok(b) => b,
            Err(e) if src.required => return Err(format!("{}: {e}", src.id)),
            Err(e) => {
                logf!("[kcd2] optional source {} missing: {e}", src.id);
                continue;
            }
        };
        match (src.id, src.kind) {
            ("zones", SourceKind::XmlRows) => {
                for r in xml_rows(&bytes, src.element)? {
                    let name = r.get("combat_zone_name").cloned().unwrap_or_default();
                    let from: f32 = r.get("angle_from").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let to: f32 = r.get("angle_to").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let id: i32 = r.get("combat_zone_id").and_then(|v| v.parse().ok()).unwrap_or(-1);
                    // A zone is on the star when KCD2 gives it a non-empty sector and the sheet knows it.
                    let Some(row) = ZONES.iter().find(|z| z.kcd2_name == name) else { continue };
                    if from != to && row.active {
                        d.zones.push(Zone { id, name, label: row.label, from, to });
                    }
                }
            }
            ("attack_types", SourceKind::XmlRows) => {
                for r in xml_rows(&bytes, src.element)? {
                    if let (Some(n), Some(c)) = (r.get("combat_attack_type_name"), r.get("relative_stamina_cost")) {
                        d.relative_stamina.insert(n.clone(), c.parse().unwrap_or(1.0));
                    }
                }
            }
            ("rpg_params", SourceKind::XmlParams) => {
                for r in xml_rows(&bytes, "rpg_param")? {
                    if r.get("rpg_param_key").map(String::as_str) == Some("BaseAttackStaminaCost") {
                        d.base_attack_stamina = r.get("rpg_param_value").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    }
                }
            }
            ("perfect_block_timing", SourceKind::XmlRows) => {
                let rows = xml_rows(&bytes, src.element)?;
                let f = |r: &HashMap<String, String>, k: &str| r.get(k).and_then(|v| v.parse::<f32>().ok());
                let mut pw = Vec::new();
                let mut rw = Vec::new();
                for r in &rows {
                    if let (Some(s), Some(h), Some(e)) = (f(r, "block_time_to_start"), f(r, "block_time_to_hit"), f(r, "block_time_to_end")) {
                        pw.push(h - s);
                        rw.push(e - h);
                    }
                }
                d.perfect_window_s = median(pw);
                d.riposte_window_s = median(rw);
            }
            (id, SourceKind::Dds) => match decode_dds(&bytes) {
                Ok(img) => {
                    d.icons.insert(src.id, img);
                }
                Err(e) => return Err(format!("{id}: {e}")),
            },
            ("sound_bank", SourceKind::FmodBank) => d.sound_bank = Some(bytes),
            (id, _) => logf!("[kcd2] source {id} has no reader"),
        }
    }
    if d.zones.is_empty() || d.base_attack_stamina <= 0.0 || d.perfect_window_s <= 0.0 {
        return Err(format!(
            "KCD2 data incomplete: {} zones, base stamina {}, perfect window {}",
            d.zones.len(), d.base_attack_stamina, d.perfect_window_s
        ));
    }
    logf!(
        "[kcd2] loaded from {}: zones [{}], stamina base {} rel {:?}, perfect window {:.3}s, riposte window {:.3}s, icons {}, sound bank {} bytes",
        root.display(),
        d.zones.iter().map(|z| format!("{}({}..{})", z.name, z.from, z.to)).collect::<Vec<_>>().join(", "),
        d.base_attack_stamina,
        d.relative_stamina,
        d.perfect_window_s,
        d.riposte_window_s,
        d.icons.len(),
        d.sound_bank.as_ref().map_or(0, |b| b.len())
    );
    Ok(d)
}

/// Zone containing a screen angle (degrees, 0 = right, 90 = up).
pub fn zone_at(zones: &[Zone], deg: f32) -> Option<&Zone> {
    zones.iter().find(|z| z.contains(deg))
}
