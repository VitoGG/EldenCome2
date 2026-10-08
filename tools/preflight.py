"""Preflight: lay every sheet over the others and over the player's own KCD2 install.

Two kinds of findings:
  BLOCKING   - an unfilled cell or a reference that does not resolve. Code generation refuses to run.
  UNVERIFIED - a cell that still needs evidence from the running game. A release refuses to ship.

usage: python -I tools/preflight.py [--kcd2 DIR] [--events FILE] [--json OUT]
"""
import argparse
import json
import os
import re
import struct
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHEETS = ROOT / "sheets"
DEFAULT_KCD2 = os.environ.get("KCT_KCD2_DIR", r"V:\Steam\steamapps\common\KingdomComeDeliverance2")
DEFAULT_EVENTS = r"V:\tools\er-ct\CheatTable\CheatEntries\Open - The Grand Archives - Elden Ring\Scripts\Functions\PlayAnimation\List.cea"

blocking, unverified, notes = [], [], []


def load(name):
    return json.loads((SHEETS / f"{name}.json").read_text(encoding="utf-8"))


def block(sheet, row, col, msg):
    blocking.append(f"{sheet}[{row}].{col}: {msg}")


def unver(sheet, row, col, msg):
    unverified.append(f"{sheet}[{row}].{col}: {msg}")


def check_filled(sheet):
    """Every row must carry every declared column, none of them null or empty (except verification columns)."""
    s = load(sheet)
    cols = list(s["columns"].keys())
    for r in s["rows"]:
        rid = r.get("id") or "/".join(str(r.get(c)) for c in cols[:3])
        for c in cols:
            if c not in r:
                block(sheet, rid, c, "missing")
            elif r[c] is None and not (c == "verified" or c.startswith("measured_")):
                block(sheet, rid, c, "unfilled")
            elif r[c] == "" or r[c] == []:
                if not (sheet == "kcd2_sources" and c == "fields" and r.get("kind") == "dds"):
                    block(sheet, rid, c, "empty")
        extra = set(r) - set(cols)
        if extra:
            block(sheet, rid, ",".join(sorted(extra)), "column not declared in the sheet")
    return s


def kcd2_read(kcd2, pak, entry):
    p = Path(kcd2) / "Data" / pak
    if not p.exists():
        return None, f"{p} not found"
    with zipfile.ZipFile(p) as z:
        try:
            return z.read(entry), None
        except KeyError:
            return None, f"{entry} not in {pak}"


def fsb5_names(data):
    i = data.find(b"FSB5")
    if i < 0:
        return None
    ver, nsamp, shdr, nsz, dsz, mode = struct.unpack_from("<IIIIII", data, i + 4)
    names_off = i + 0x3C + shdr
    offs = struct.unpack_from(f"<{nsamp}I", data, names_off)
    names = set()
    for o in offs:
        a = names_off + o
        names.add(data[a:data.index(b"\0", a)].decode("latin1"))
    return names, mode


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kcd2", default=DEFAULT_KCD2)
    ap.add_argument("--events", default=DEFAULT_EVENTS)
    ap.add_argument("--json")
    a = ap.parse_args()

    names = ["kcd2_sources", "zones", "attacks", "weapon_classes", "swing_map", "inputs", "defense",
             "enemy_dirs", "hud", "sounds", "hooks"]
    S = {n: check_filled(n) for n in names}
    ids = {n: {r.get("id") for r in S[n]["rows"]} for n in names}

    # ---- KCD2 sources against this PC's copy
    src_data = {}
    for r in S["kcd2_sources"]["rows"]:
        data, err = kcd2_read(a.kcd2, r["pak"], r["entry"])
        if err:
            (block if r["required"] else unver)("kcd2_sources", r["id"], "entry", err)
            continue
        src_data[r["id"]] = data
        if r["kind"] == "xml_rows":
            t = data.decode("utf-8", "replace")
            for f in r["fields"]:
                if not re.search(rf'<{r["element"]}\b[^>]*\b{f}="', t):
                    block("kcd2_sources", r["id"], "fields", f"attribute {f} not found on <{r['element']}>")
        elif r["kind"] == "xml_params":
            t = data.decode("utf-8", "replace")
            for f in r["fields"]:
                if f'{r["element"]}="{f}"' not in t:
                    block("kcd2_sources", r["id"], "fields", f"param {f} not found")
        elif r["kind"] == "dds":
            if data[:4] != b"DDS ":
                block("kcd2_sources", r["id"], "entry", "not a DDS file")
            else:
                h, w = struct.unpack_from("<II", data, 12)
                fourcc = data[84:88]
                dx10 = struct.unpack_from("<I", data, 128)[0] if fourcc == b"DX10" else None
                notes.append(f"{r['id']}: {w}x{h} {fourcc.decode('latin1')}{'' if dx10 is None else ' dxgi=' + str(dx10)}")
        elif r["kind"] == "fmod_bank":
            got = fsb5_names(data)
            if not got:
                block("kcd2_sources", r["id"], "element", "no FSB5 in bank")
            else:
                have, mode = got
                notes.append(f"{r['id']}: FSB5 codec {mode}, {len(have)} samples")
                for f in r["fields"]:
                    if f not in have:
                        block("kcd2_sources", r["id"], "fields", f"sample {f} not in bank")

    # ---- zones against KCD2
    if "zones" in src_data:
        t = src_data["zones"].decode()
        live = {}
        for m in re.finditer(r"<combat_zone ([^>]*)/>", t):
            at = dict(re.findall(r'(\w+)="([^"]*)"', m.group(1)))
            live[at["combat_zone_name"]] = at
        for r in S["zones"]["rows"]:
            z = live.get(r["kcd2_name"])
            if not z:
                block("zones", r["kcd2_name"], "kcd2_name", "not in KCD2 combat_zone.xml")
                continue
            if int(z["combat_zone_id"]) != r["kcd2_zone_id"]:
                block("zones", r["kcd2_name"], "kcd2_zone_id", f"KCD2 says {z['combat_zone_id']}")
            for c, k in (("expected_angle_from", "angle_from"), ("expected_angle_to", "angle_to")):
                if float(z[k]) != float(r[c]):
                    block("zones", r["kcd2_name"], c, f"KCD2 says {z[k]}")
            if (float(z["angle_from"]) != float(z["angle_to"])) != r["active"]:
                block("zones", r["kcd2_name"], "active", "disagrees with KCD2 angles")
    active_zones = {r["kcd2_name"] for r in S["zones"]["rows"] if r["active"]}

    if "attack_types" in src_data:
        t = src_data["attack_types"].decode()
        kinds = set(re.findall(r'combat_attack_type_name="([^"]+)"', t))
        for r in S["attacks"]["rows"]:
            if r["kcd2_attack_type"] not in kinds:
                block("attacks", r["id"], "kcd2_attack_type", "not in KCD2 combat_attack_type.xml")

    # ---- cross-sheet references
    for r in S["attacks"]["rows"]:
        if r["input"] not in ids["inputs"]:
            block("attacks", r["id"], "input", f"{r['input']} is not an inputs.id")

    events = set()
    if Path(a.events).exists():
        events = set(re.findall(r'"(W_[A-Za-z0-9_]+)"', Path(a.events).read_text(encoding="utf-8", errors="replace")))
    else:
        unver("swing_map", "*", "event_*", f"event list {a.events} not found; event names unchecked")

    covered = set()
    for r in S["swing_map"]["rows"]:
        rid = f"{r['weapon_class']}/{r['attack']}/{r['kcd2_zone']}"
        if r["weapon_class"] not in ids["weapon_classes"]:
            block("swing_map", rid, "weapon_class", "not a weapon_classes.id")
        if r["attack"] not in ids["attacks"]:
            block("swing_map", rid, "attack", "not an attacks.id")
        if r["kcd2_zone"] != "-" and r["kcd2_zone"] not in active_zones:
            block("swing_map", rid, "kcd2_zone", "not an active zone")
        for c in ("event_1h", "event_2h"):
            if events and r[c] not in events:
                block("swing_map", rid, c, f"{r[c]} is not an Elden Ring behavior event")
        want = r["kcd2_zone"] if r["kcd2_zone"] != "-" else "thrust"
        for c in ("measured_zone_1h", "measured_zone_2h"):
            if r[c] is None:
                unver("swing_map", rid, c, "not measured in the swing lab")
            elif r[c] != want:
                block("swing_map", rid, c, f"measured {r[c]}, row says {want}: pick another event")
        covered.add((r["weapon_class"], r["attack"], r["kcd2_zone"]))
    uses_zone = {r["id"]: r["uses_zone"] for r in S["attacks"]["rows"]}
    for wc in ids["weapon_classes"]:
        for atk, zoned in uses_zone.items():
            need = active_zones if zoned else {"-"}
            for z in need:
                if (wc, atk, z) not in covered:
                    block("swing_map", f"{wc}/{atk}/{z}", "row", "missing")

    for r in S["weapon_classes"]["rows"]:
        if not r["wep_type_verified"]:
            unver("weapon_classes", r["id"], "wep_type_verified", f"wepType {r['er_wep_type']} not seen in the game log")

    hook_ids = ids["hooks"] | {"game_default"}
    for r in S["defense"]["rows"]:
        for tok in re.findall(r"[a-z_]+", r["er_mechanism"].split("(")[0]):
            if tok not in hook_ids:
                block("defense", r["id"], "er_mechanism", f"{tok} is not a hooks.id")
        if r["sound"] != "-" and r["sound"] not in ids["sounds"]:
            block("defense", r["id"], "sound", f"{r['sound']} is not a sounds.id")
        m = re.match(r"([a-z_]+):", r["kcd2_value"])
        if r["kcd2_value"] != "-" and (not m or m.group(1) not in ids["kcd2_sources"]):
            block("defense", r["id"], "kcd2_value", "must start with a kcd2_sources id")
        for ev in re.findall(r"W_[A-Za-z0-9_]+", r["er_mechanism"]):
            if events and ev not in events:
                block("defense", r["id"], "er_mechanism", f"{ev} is not an Elden Ring behavior event")

    for r in S["hud"]["rows"]:
        if r["icon"] != "-" and r["icon"] not in ids["kcd2_sources"]:
            block("hud", r["id"], "icon", f"{r['icon']} is not a kcd2_sources id")

    bank_fields = {f for r in S["kcd2_sources"]["rows"] if r["id"] == "sound_bank" for f in r["fields"]}
    for r in S["sounds"]["rows"]:
        if r["source"] not in ids["kcd2_sources"]:
            block("sounds", r["id"], "source", "not a kcd2_sources id")
        for smp in r["samples"]:
            if smp not in bank_fields:
                block("sounds", r["id"], "samples", f"{smp} not listed in kcd2_sources.sound_bank.fields")

    for r in S["enemy_dirs"]["rows"]:
        if not r["verified"]:
            unver("enemy_dirs", r["id"], "verified", "not confirmed in game")
    for r in S["hooks"]["rows"]:
        if not r["verified"]:
            unver("hooks", r["id"], "verified", "no log evidence yet")

    total_cells = sum(len(S[n]["rows"]) * len(S[n]["columns"]) for n in names)
    print(f"Preflight: {len(names)} sheets, {sum(len(S[n]['rows']) for n in names)} rows, {total_cells} cells")
    for n in notes:
        print("  note:", n)
    print(f"\nBLOCKING ({len(blocking)})")
    for b in blocking:
        print("  -", b)
    print(f"\nUNVERIFIED ({len(unverified)})")
    for u in unverified:
        print("  -", u)
    if a.json:
        Path(a.json).write_text(json.dumps({"blocking": blocking, "unverified": unverified, "notes": notes}, indent=1))
    sys.exit(1 if blocking else 0)


if __name__ == "__main__":
    main()
