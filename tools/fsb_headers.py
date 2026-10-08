"""Collect the Vorbis setup headers KCD2's FMOD bank needs and write them to mod/assets/vorbis_setup.bin.

FMOD strips the Vorbis setup header from each sample and stores only its CRC32; the decoder looks the
header up in a table. The table comes from python-fsb5 (MIT, (c) 2016 Simon Pinfold) — these are encoder
codebooks, not game audio. Only the CRCs used by the player's bank (this PC's copy) are kept.

Output format: u32 count, then per header: u32 crc32, u32 length, bytes.
usage: python -I tools/fsb_headers.py [--kcd2 DIR] [--fsb5 V:/tools/python-fsb5]
"""
import argparse
import ast
import struct
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def sample_crcs(bank: bytes):
    i = bank.find(b"FSB5")
    ver, nsamp, shdr, nsz, dsz, mode = struct.unpack_from("<IIIIII", bank, i + 4)
    o = i + 0x3C
    crcs = {}
    for _ in range(nsamp):
        (hdr,) = struct.unpack_from("<Q", bank, o)
        o += 8
        nxt = hdr & 1
        while nxt:
            (ch,) = struct.unpack_from("<I", bank, o)
            nxt = ch & 1
            size = (ch >> 1) & 0xFFFFFF
            ctype = (ch >> 25) & 0x7F
            if ctype == 11:  # VORBISDATA
                (crc,) = struct.unpack_from("<I", bank, o + 4)
                crcs[crc] = crcs.get(crc, 0) + 1
            o += 4 + size
    return crcs, mode


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kcd2", default=r"V:\Steam\steamapps\common\KingdomComeDeliverance2")
    ap.add_argument("--fsb5", default=r"V:\tools\python-fsb5")
    a = ap.parse_args()
    with zipfile.ZipFile(Path(a.kcd2) / "Data" / "Sounds.pak") as z:
        bank = z.read("Sounds/fmod/Build/pc/kcd.assets.bank")
    crcs, mode = sample_crcs(bank)
    print(f"bank codec {mode}, {sum(crcs.values())} vorbis samples, {len(crcs)} distinct setup headers")
    src = (Path(a.fsb5) / "fsb5" / "vorbis_headers.py").read_text(encoding="utf-8")
    table = ast.literal_eval(src[src.index("{"):src.rindex("}") + 1])
    out = bytearray(struct.pack("<I", 0))
    n = 0
    for crc in sorted(crcs):
        if crc not in table:
            print(f"  missing header for crc {crc} ({crcs[crc]} samples)")
            continue
        h = table[crc]
        out += struct.pack("<II", crc, len(h)) + h
        n += 1
    struct.pack_into("<I", out, 0, n)
    dst = ROOT / "mod" / "assets" / "vorbis_setup.bin"
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_bytes(out)
    print(f"wrote {n} headers, {len(out)} bytes -> {dst.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
