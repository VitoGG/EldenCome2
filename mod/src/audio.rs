//! sounds sheet + hooks.audio_out: KCD2's own clash sounds, decoded at load from the FMOD bank in the
//! player's KCD2 install (kcd2_sources.sound_bank), played with Windows' PlaySound from memory.
//!
//! FMOD stores Vorbis with its setup header stripped (only a CRC32 is kept). The two setup headers this
//! bank uses are embedded from python-fsb5's table (MIT, (c) 2016 Simon Pinfold) by tools/fsb_headers.py;
//! they are encoder codebooks, not audio. Decoding: lewton (MIT/Apache-2.0).

use std::collections::HashMap;
use std::sync::Mutex;

use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::core::PCWSTR;

use crate::generated::SOUNDS;
use crate::logf;

static SETUP_HEADERS: &[u8] = include_bytes!("../assets/vorbis_setup.bin");

struct Bank {
    /// sounds.id -> ready-to-play WAV files
    wavs: HashMap<&'static str, Vec<Vec<u8>>>,
    seed: u32,
}

static BANK: Mutex<Option<Bank>> = Mutex::new(None);

fn setup_header(crc: u32) -> Option<&'static [u8]> {
    let n = u32::from_le_bytes(SETUP_HEADERS[0..4].try_into().ok()?) as usize;
    let mut o = 4;
    for _ in 0..n {
        let c = u32::from_le_bytes(SETUP_HEADERS[o..o + 4].try_into().ok()?);
        let len = u32::from_le_bytes(SETUP_HEADERS[o + 4..o + 8].try_into().ok()?) as usize;
        if c == crc {
            return Some(&SETUP_HEADERS[o + 8..o + 8 + len]);
        }
        o += 8 + len;
    }
    None
}

struct FsbSample {
    name: String,
    rate: u32,
    channels: u8,
    crc: u32,
    data: std::ops::Range<usize>,
}

fn parse_fsb5(bank: &[u8]) -> Option<Vec<FsbSample>> {
    let i = bank.windows(4).position(|w| w == b"FSB5")?;
    let rd32 = |o: usize| u32::from_le_bytes(bank[o..o + 4].try_into().unwrap());
    let (nsamp, shdr, nsz, dsz) = (rd32(i + 8) as usize, rd32(i + 12) as usize, rd32(i + 16) as usize, rd32(i + 20) as usize);
    let mut o = i + 0x3C;
    let mut heads = Vec::with_capacity(nsamp);
    for _ in 0..nsamp {
        let h = u64::from_le_bytes(bank[o..o + 8].try_into().ok()?);
        o += 8;
        let freq_idx = (h >> 1) & 0xF;
        let channels = ((h >> 5) & 1) as u8 + 1;
        let off = ((h >> 6) & 0x0FFF_FFFF) as usize * 16;
        let mut rate = match freq_idx {
            1 => 8000, 2 => 11000, 3 => 11025, 4 => 16000, 5 => 22050, 6 => 24000, 7 => 32000, 8 => 44100, 9 => 48000, _ => 44100,
        };
        let mut crc = 0;
        let mut next = h & 1;
        while next != 0 {
            let c = rd32(o);
            next = (c & 1) as u64;
            let size = ((c >> 1) & 0xFF_FFFF) as usize;
            let kind = (c >> 25) & 0x7F;
            match kind {
                2 => rate = rd32(o + 4),
                11 => crc = rd32(o + 4),
                _ => {}
            }
            o += 4 + size;
        }
        heads.push((rate, channels, crc, off));
    }
    let names_at = i + 0x3C + shdr;
    let data_at = names_at + nsz;
    let mut out = Vec::with_capacity(nsamp);
    for (k, (rate, channels, crc, off)) in heads.iter().enumerate() {
        let no = names_at + rd32(names_at + k * 4) as usize;
        let end = bank[no..].iter().position(|b| *b == 0).map_or(no, |p| no + p);
        let name = String::from_utf8_lossy(&bank[no..end]).into_owned();
        let stop = heads.get(k + 1).map_or(dsz, |h| h.3);
        out.push(FsbSample { name, rate: *rate, channels: *channels, crc: *crc, data: data_at + off..data_at + stop });
    }
    Some(out)
}

fn decode(bank: &[u8], s: &FsbSample) -> Result<Vec<i16>, String> {
    let setup_bytes = setup_header(s.crc).ok_or(format!("no setup header for crc {:#x}", s.crc))?;
    // Rebuild the Vorbis identification and comment headers FMOD leaves out (blocksizes 256 / 2048).
    let mut ident = vec![0x01];
    ident.extend_from_slice(b"vorbis");
    ident.extend_from_slice(&0u32.to_le_bytes());
    ident.push(s.channels);
    ident.extend_from_slice(&s.rate.to_le_bytes());
    ident.extend_from_slice(&[0u8; 12]);
    ident.push(0xB8); // log2 blocksizes: 8 (short) | 11 (long) << 4
    ident.push(0x01);
    let mut comment = vec![0x03];
    comment.extend_from_slice(b"vorbis");
    comment.extend_from_slice(&0u32.to_le_bytes());
    comment.extend_from_slice(&0u32.to_le_bytes());
    comment.push(0x01);
    let id = lewton::header::read_header_ident(&ident).map_err(|e| format!("ident: {e:?}"))?;
    lewton::header::read_header_comment(&comment).map_err(|e| format!("comment: {e:?}"))?;
    let setup = lewton::header::read_header_setup(setup_bytes, id.audio_channels, (id.blocksize_0, id.blocksize_1)).map_err(|e| format!("setup: {e:?}"))?;
    let mut pwr = lewton::audio::PreviousWindowRight::new();
    let data = &bank[s.data.clone()];
    let mut o = 0;
    let mut pcm = Vec::new();
    while o + 2 <= data.len() {
        let len = u16::from_le_bytes([data[o], data[o + 1]]) as usize;
        o += 2;
        if len == 0 || o + len > data.len() {
            break;
        }
        let chans = lewton::audio::read_audio_packet(&id, &setup, &data[o..o + len], &mut pwr).map_err(|e| format!("packet: {e:?}"))?;
        o += len;
        if let Some(first) = chans.first() {
            for f in 0..first.len() {
                for c in &chans {
                    pcm.push(c[f]);
                }
            }
        }
    }
    Ok(pcm)
}

fn wav(pcm: &[i16], rate: u32, channels: u8, volume: f32) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&(channels as u16).to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    w.extend_from_slice(&(channels as u16 * 2).to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&((*s as f32 * volume) as i16).to_le_bytes());
    }
    w
}

/// Decode every sample the sounds sheet names (background thread at boot).
pub fn init(bank: &[u8]) {
    let Some(samples) = parse_fsb5(bank) else {
        logf!("[hook:audio_out] ERROR: no FSB5 in KCD2's bank");
        return;
    };
    let mut wavs: HashMap<&'static str, Vec<Vec<u8>>> = HashMap::new();
    let mut ok = 0;
    for row in SOUNDS {
        for name in row.samples {
            match samples.iter().find(|s| s.name == *name) {
                Some(s) => match decode(bank, s) {
                    Ok(pcm) => {
                        ok += 1;
                        wavs.entry(row.id).or_default().push(wav(&pcm, s.rate, s.channels, row.volume));
                    }
                    Err(e) => logf!("[hook:audio_out] {name}: {e}"),
                },
                None => logf!("[hook:audio_out] {name} not in KCD2's bank"),
            }
        }
    }
    logf!("[hook:audio_out] decoded {ok} KCD2 samples for {} sounds", wavs.len());
    *BANK.lock().unwrap() = Some(Bank { wavs, seed: 0x9E37_79B9 });
}

pub fn play(id: &str) {
    let mut g = BANK.lock().unwrap();
    let Some(b) = g.as_mut() else {
        logf!("[sound] {id} (KCD2 sounds not loaded)");
        return;
    };
    let Some(list) = b.wavs.get(id).filter(|l| !l.is_empty()) else { return };
    b.seed = b.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let w = &list[(b.seed >> 16) as usize % list.len()];
    // The WAV stays alive in BANK for the whole session, as SND_MEMORY | SND_ASYNC requires.
    let ok = unsafe { PlaySoundW(PCWSTR(w.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
    crate::log::once(&format!("snd-{id}"), &format!("[hook:audio_out] played {id}: {}", ok.as_bool()));
}
