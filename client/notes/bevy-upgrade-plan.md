# Bevy upgrade plan (0.16 → 0.20, one version at a time)

## Why this plan exists

The first attempt (2026-11-17) jumped Bevy 0.16 → 0.20 and lightyear 0.23 →
0.31 in one change across ~150 files. It compiled, but:

- every real bug only showed up when the game was run (`cargo check` can't
  catch them),
- it ran about **2x slower** (avg ~50 FPS vs ~111, dev builds both times; ~55
  vs ~130 FPS even with **no** enemies, so a fixed per-frame cost, not the
  horde),
- zombies played the kill sound and died, then got back up,
- with four Bevy versions in one diff there was no way to tell which version
  caused what.

That work is shelved on the branch **`bevy-0.20-upgrade`** (commit `f1b25e2`).
`main` stays on 0.16. The branch is a reference for *how* each API was
migrated, never something to merge.

## Rules for every step

1. **One Bevy minor version per step**: 0.17, then 0.18, then 0.19, then 0.20.
   Each step gets its own branch off `main` (`bevy-0.17`, …).
2. **Bump only what that version requires**: Bevy, plus the matching versions
   of lightyear, `bevy_egui` and the collision crate. Check each one's
   changelog/README for the release that matches that Bevy first. If one hasn't
   released for that Bevy yet, the step waits.
3. **Make only the changes needed to compile.** Follow the official migration
   guide for that version (bevy.org/learn/migration-guides). Deprecation
   warnings are fine to leave. Do **not** adopt new features in the same step,
   e.g.:
   - `Interaction` → `Hovered` / `Pressed`
   - `.wgsl` → `.wesl` shaders
   - `ScreenSpaceTransmission` (screen-space refraction) for the water. This
     is the top suspect for the 2x slowdown.

   Each of those becomes its own small change *after* the upgrade, measured on
   its own.
4. **Gate before merging** (see "Checks" below): compile + tests, smoke
   checklist, perf benchmark.
5. **Merge into `main` only if it passes and performance is within
   run-to-run noise.** `main` always works, and it's fine to stop at any
   version and keep what's been gained so far.
6. If a step gets slower or breaks, only that one version's changes need
   investigating.

## Step 0: prep on 0.16 (before upgrading anything)

1. **Autostart shortcut.** A dev env var such as
   `TRICKSHOT_AUTOSTART=zombies,dertempel_night,21` that creates the lobby,
   starts it and drops straight into the match. That way a match can be
   launched and screenshotted without clicking through menus, so runtime bugs
   are caught before they reach a playtest. (Claude still asks permission
   before launching the client.)
2. **Smoke checklist** (below), kept in this file.
3. **Baseline perf with the same build type every time.** Use `--release` for
   both client and server from now on. Record 3 baseline runs on 0.16 using
   the benchmark in `security-performance.md` and copy the reports from
   `~/.config/bevy-trickshot/perf/` into `perf/bevy-0.16/` with the commit
   hash.

## Steps 1–4: 0.17, 0.18, 0.19, 0.20

For each version:

1. Branch off `main`: `git switch -c bevy-0.1X`.
2. Bump Bevy + matching lightyear / `bevy_egui` / collision crate (workspace
   `Cargo.toml`, `client/Cargo.toml`, `server/Cargo.toml`).
3. Fix compile errors only, using the migration guide. For any file,
   `git diff main bevy-0.20-upgrade -- <file>` shows how the shelved attempt
   handled it.
4. Run the checks below.
5. Merge into `main`, bump the patch version + add a changelog line only if
   something player-facing changed.
6. Stop and let the user playtest before starting the next version.

## Checks (every step)

1. `cargo check -p bevy-trickshot -p server -p shared` and
   `cargo test -p shared`.
2. **Smoke checklist** (about 5 minutes, debug mode on):
   - [ ] Client starts, main menu loads (no duplicate-component panic)
   - [ ] Connects to a local server (server log shows a connection, not
         "ignored connection request")
   - [ ] Lobby: create, change mode/map, leave popup, mouse back button
   - [ ] Start each mode: Freestyle, Free For All, Zombies, Search and Destroy
   - [ ] The 3D world is visible in the match (not a frozen loading screen)
   - [ ] HUD elements don't smear/stack (round counter slide, damage tint)
   - [ ] A shot zombie **stays dead**
   - [ ] Scope in/out (scope camera + reticle)
   - [ ] Pack-a-Punch, Wunderfizz, crafting menus open and close
   - [ ] Level editor opens and a map loads
   - [ ] Leave the match, start another: nothing carried over
3. **Perf benchmark**: the routine in `security-performance.md`, 3 runs,
   compare the middle run against the previous version's middle run (avg,
   1% low, 0.1% low, 30+ enemies, peak draw calls). Note: on the shelved 0.20
   branch the ENTITIES readout stuck at exactly 9200, so check that the
   counter itself still works before trusting entity numbers.

## Known traps from the shelved 0.20 attempt

Handle each at the step that introduces it:

| Problem | Fix used on the shelved branch | Likely step |
|---|---|---|
| `Button` removed/deprecated; replacing it with `Interaction::default(), FocusPolicy::Block` doubled up where an `Interaction::default()` already followed → runtime panic "Bundle … has duplicate components" | remove the duplicate `Interaction` line | whichever version deprecates `Button` |
| lightyear rejects every connection: "server address not in connect token whitelist" (server binds `[::]`, client token says `127.0.0.1`; on fly.io the bind address never matches the public IP) | `server_addr_check: false` in `server/src/net.rs`'s `NetcodeConfig` (clients mint their own tokens, so it adds no security) | the lightyear bump that adds the check |
| 3D world never shows; last loading-screen frame stays frozen, HUD bits smear and stack, red tint accumulates | HUD camera `clear_color: ClearColorConfig::Custom(Color::NONE)` in `hud/setup.rs` (each camera got its own intermediate texture, so `None` never cleared it) | whichever version changes camera intermediate textures |
| Zombies die then get back up | **not found yet** | unknown, so the smoke checklist checks it every step |
| ~2x slower even with no enemies | **not found yet**; suspect `ScreenSpaceTransmission` on the world camera | unknown, so the per-step benchmark will point to it |
