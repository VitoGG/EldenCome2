"""Stage a local copy laid out exactly like Melty installs it, for testing:
  local/managed/modengine2/   <- ModEngine2 release-2.1.0 (what Melty installs)
  local/managed/kctarnished/  <- the release files + kctarnished.env (what Melty's settings line writes)

usage: python -I tools/stage_local.py [--me2 DIR] [--kcd2 DIR]
Then run local/play.cmd (KCT_LAB=1 for the swing lab).
"""
import argparse
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DLL = ROOT / "mod" / "target" / "x86_64-pc-windows-msvc" / "release" / "kctarnished.dll"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--me2", default=r"V:\tools\modengine2")
    ap.add_argument("--kcd2", default=r"V:\Steam\steamapps\common\KingdomComeDeliverance2")
    a = ap.parse_args()
    managed = ROOT / "local" / "managed"
    me2 = managed / "modengine2"
    if not me2.exists():
        shutil.copytree(a.me2, me2)
    dst = managed / "kctarnished"
    dst.mkdir(parents=True, exist_ok=True)
    shutil.copy2(DLL, dst / "kctarnished.dll")
    for f in (ROOT / "package").iterdir():
        shutil.copy2(f, dst / f.name)
    (dst / "kctarnished.env").write_text(f"KCT_KCD2_DIR={Path(a.kcd2).as_posix()}\n", encoding="utf-8")
    play = ROOT / "local" / "play.cmd"
    play.write_text(
        "@echo off\r\n"
        "rem Same command Melty runs on Play (plus KCT_LAB for the swing lab when set by the caller).\r\n"
        f'set KCT_KCD2_DIR={a.kcd2}\r\n'
        f'set KCT_LAB_DIR={ROOT / "local" / "lab"}\r\n'
        f'cd /d "{me2}"\r\n'
        f'"{me2 / "modengine2_launcher.exe"}" -t er -c "{dst / "config_kctarnished.toml"}"\r\n',
        encoding="utf-8",
    )
    print(f"staged {dst} ({', '.join(sorted(p.name for p in dst.iterdir()))})")
    print(f"run: {play}")


if __name__ == "__main__":
    main()
