"""Fill swing_map from swing-lab measurements (local/lab/lab_results.jsonl, written by the mod's lab).

For each row and grip: keep the current event if the lab measured it in the wanted zone; otherwise pick the
measured event that lands in the wanted zone (light attacks first, then the earliest peak). The measured
zone goes into measured_zone_1h / measured_zone_2h. Rows without lab data for their class are untouched.
usage: python -I tools/lab_apply.py [results.jsonl]
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
res = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "local" / "lab" / "lab_results.jsonl")
meas = {}
for line in res.read_text(encoding="utf-8").splitlines():
    if not line.strip():
        continue
    r = json.loads(line)
    if r["peak"] == "no_bone":
        continue
    meas[(r["weapon_class"], r["grip"], r["event"])] = (r["peak"], r.get("peak_t", 9.9))

p = ROOT / "sheets" / "swing_map.json"
sheet = json.loads(p.read_text(encoding="utf-8"))
changed = 0
for row in sheet["rows"]:
    want = "thrust" if row["kcd2_zone"] == "-" else row["kcd2_zone"]
    for grip, ev_col, m_col in (("1h", "event_1h", "measured_zone_1h"), ("2h", "event_2h", "measured_zone_2h")):
        cur = meas.get((row["weapon_class"], grip, row[ev_col]))
        if cur and cur[0] == want:
            if row[m_col] != want:
                row[m_col] = want
                changed += 1
            continue
        cands = [(ev, pk, t) for (c, g, ev), (pk, t) in meas.items() if c == row["weapon_class"] and g == grip and pk == want]
        if not cands:
            if cur:
                row[m_col] = cur[0]  # measured, but wrong: preflight will flag it
                changed += 1
            continue
        cands.sort(key=lambda x: ("Heavy" in x[0], x[2]))
        ev = cands[0][0]
        print(f"{row['weapon_class']} {grip} {row['attack']}/{row['kcd2_zone']}: {row[ev_col]} -> {ev} (measured {want} at {cands[0][2]:.2f}s)")
        row[ev_col] = ev
        row[m_col] = want
        changed += 1
p.write_text(json.dumps(sheet, indent=1), encoding="utf-8")
print(f"swing_map: {changed} cells updated")
