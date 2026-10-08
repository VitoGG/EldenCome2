# MODLOG: Kingdom Come: Tarnished

Kingdom Come: Deliverance II's directional sword combat inside Elden Ring. Elden Ring is the host (primary);
KCD2 is a companion whose own files (combat tables, HUD icons, clash sounds) are read from the player's
install at runtime. Solo, offline (Melty starts ER through ModEngine2 with EAC off).

## Versions and paths (this PC)
- Elden Ring exe 2.7.1.0 — `V:\Steam\steamapps\common\ELDEN RING\Game`
- KCD2 exe 1.5.6.0 — `V:\Steam\steamapps\common\KingdomComeDeliverance2`
- ModEngine2 release-2.1.0 (what Melty installs) — local copy `V:\tools\modengine2`
- fromsoftware-rs 0.14.0 @ 59fbd3b — `V:\tools\fromsoftware-rs` (eldenring crate, RVA table `Ww2710`)
- The Grand Archives ER CT 1.14.0 — `V:\tools\er-ct` (PlayAnimation AOB, behavior event names)
- universal-modder — `V:\tools\universal-modder`
- Rust 1.97.1 msvc, VS 2022 (MSVC 14.41), CMake, Python 3.13

## Route (and why)
Native hook DLL in Rust, loaded by ModEngine2's `external_dlls` (Melty installs ModEngine2 itself; the
recipe points the launcher at our own toml). ER has no scripting API; this is the route the working
CS2×ER note in the universal-modder KB used (fromsoftware-rs + hudhook ImGui overlay).
- Recipe draft passed Melty `one_click_check` = yes (ModEngine2 automatic, KCD2 companion folder passed in
  `KCT_KCD2_DIR` env + `{managed}/kctarnished/kctarnished.env`).
- ModEngine2 toml: relative `external_dlls` paths resolve against the **launcher's** folder
  (`{managed}/modengine2`), so our toml uses `..\\kctarnished\\kctarnished.dll`.

## Facts learned
- KCD2 `Data/Tables.pak` is a zip. `Libs/Tables/combat/combat_zone.xml`: 4 active 90° sectors —
  head 45–135, upper_left 135–225, lower 225–315, upper_right 315–45; lower_left/lower_right have 0–0 (unused).
- `combat_attack_type.xml`: stab rel. stamina 1.0, slash 1.2, smash 1.2. `rpg_param.xml`: BaseAttackStaminaCost 26.
- `combat_action_perfect_block.xml`: 206 rows with block_time_to_start / block_time_to_hit / block_time_to_end.
- KCD2 HUD icons: `Data/IPL_GameData.pak` → `Libs/UI/Textures/Icons/Tutorials/combat_cursor_icon.dds`,
  `combat_pb_cursor_icon.dds`, `combat_riposta_icon.dds`.
- KCD2 sounds: `Data/Sounds.pak` → `Sounds/fmod/Build/pc/kcd.assets.bank` (FMOD bank, FSB5 inside, codec 15 =
  Vorbis, 4705 samples). Named samples: `sword_sword_clash44..64`, `sword_hit_sword_01..03`,
  `sword_sword_clinch3_*`, `sword_hit_armor_*`.
- ER: fire a behavior event on a character = fn at AOB `74 ?? 48 85 d2 74 ?? 48 8d 4c 24 50` − 0xD,
  args (hkbCharacter*, const wchar_t* name); hkbCharacter = *(*(*(ChrIns+0x190)+0x28)+0x10)+0x30
  (TGA CT). Events: W_AttackRightLight1..6, W_AttackRightHeavy1Start/2Start, W_AttackRightBackstep,
  W_AttackRightLightStep, W_AttackRightLightDash, W_AttackRightHeavyCounter, W_AttackBoth* (two-handed)…
- ER input: DirectInput `GetDeviceState` only (KB note). `CSChrActionRequestModule` holds per-frame action bits.
- `PlayerIns.locked_on_enemy` (FieldInsHandle), `ChrIns.is_locked_on`, `CSChrTimeActModule.anim_queue`
  (current anim ids, any character).

## Open / next
- Bone (hand/weapon) world positions for enemies: not in fromsoftware-rs. Probe hkbCharacter → skeleton
  bone names + pose in game.
- Verify ER wepType ids for the four sword classes; verify swing direction of every mapped event (swing lab).

## Session 1 results (2026-10-08)
- Verified in game (log evidence in kctarnished.log): boot, KCD2 load, fire_behavior_event (eldenring.exe+0xc15ab0),
  overlay with KCD2 icons, player weapon/wepType 3, bones (skeleton *(ChrModelIns+0x158), hkReferencedObject 0x18:
  parents +0x20, bones +0x30; live local pose *(ChrIns+0x398)+0x50; R_Hand 0.92 m from feet), KCD2 sounds decoded
  (FSB5 Vorbis, 2 setup headers) and played via PlaySound.
- Swing map: straight sword 1h + 2h all rows measured in the lab and re-confirmed (2h rows re-run 5/5).
- Rule change: cocked-position failed; peak screen-plane velocity over 2-frame steps (pose double-buffered),
  thrust = view-axis travel > 1.3x screen travel around the peak; enemy zones learned per (behavior, anim).
- Next: lock-on combat test vs a real enemy (cursor, suppression, wrong-side guard, perfect block, riposte),
  1h rows re-run, other sword classes (need those weapons), screenshot/clip, package + upload.
- Gotchas: PowerShell tool is sandboxed (broken junction left at C:\Users\User\AppData\Local\universal-modder);
  C: has ~85 MB free; toolkit data now in V:\tools\umlocal (LOCALAPPDATA override). Title menus need held keys.
