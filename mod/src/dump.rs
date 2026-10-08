//! Lab only: dump the pointer graph around a character to a file for offline analysis
//! (tools/analyze_dump.py). Runs on its own thread; reads are racy by nature, which is fine for probing.
//! Format: b"KCTD", u32 nroots, nroots x (u64 addr, 16-byte name), then nodes: (u64 addr, u32 len, bytes).

use std::collections::{HashSet, VecDeque};
use std::io::Write;

use crate::logf;
use crate::skeleton::readable;

pub fn spawn(roots: Vec<(&'static str, usize)>, depth: usize, span: usize, max_nodes: usize) {
    std::thread::spawn(move || {
        let path = crate::log::lab_dir().join("graph.kctd");
        let mut out = Vec::<u8>::new();
        out.extend_from_slice(b"KCTD");
        out.extend_from_slice(&(roots.len() as u32).to_le_bytes());
        for (n, a) in &roots {
            out.extend_from_slice(&(*a as u64).to_le_bytes());
            let mut name = [0u8; 16];
            for (i, b) in n.bytes().take(16).enumerate() {
                name[i] = b;
            }
            out.extend_from_slice(&name);
        }
        let mut seen = HashSet::new();
        let mut q: VecDeque<(usize, usize)> = roots.iter().map(|r| (r.1, 0)).collect();
        let mut nodes = 0;
        while let Some((a, d)) = q.pop_front() {
            if nodes >= max_nodes || !seen.insert(a) {
                continue;
            }
            let mut len = span;
            while len > 0x40 && !readable(a, len) {
                len /= 2;
            }
            if !readable(a, len) {
                continue;
            }
            let bytes = unsafe { std::slice::from_raw_parts(a as *const u8, len) }.to_vec();
            nodes += 1;
            if d < depth {
                for off in (0..len).step_by(8) {
                    let p = u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap()) as usize;
                    if p >= 0x10000 && p <= 0x7FFF_FFFF_FFFF && p % 8 == 0 && !seen.contains(&p) && readable(p, 0x40) {
                        q.push_back((p, d + 1));
                    }
                }
            }
            out.extend_from_slice(&(a as u64).to_le_bytes());
            out.extend_from_slice(&(len as u32).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        match std::fs::File::create(&path).and_then(|mut f| f.write_all(&out)) {
            Ok(()) => logf!("[dump] {} nodes, {} bytes -> {}", nodes, out.len(), path.display()),
            Err(e) => logf!("[dump] write failed: {e}"),
        }
    });
}
