# bevy-trickshot

## What this game is

A multiplayer 3D trickshot game, Call-of-Duty-style, built in Rust end to end
(client + server + shared protocol crate — see the repo-root `README.md` for
the workspace layout). Players sniper each other (or bots) and score style
points for how they get the kill — no-scopes, spins, wallbangs, collaterals,
headshots — not just for the kill itself (see `shared/src/scoring.rs` and the
`# Scoring Points` section of `client/notes/todo.md`). There are currently two
`GameMode`s (`shared/src/protocol.rs`): **Freestyle** (free-for-all against
bots, highest score wins) and **Free For All** (real PvP, first to the kill
limit or most kills when time runs out). The client self-updates silently on
every launch (`client/src/updater.rs`), so the changelog panel below is the
only place a player learns anything changed.

## Working in this repo

- `client/notes/todo.md` is the backlog. `# Added` is unsorted/incoming work;
  `# Done` and the other headed sections below it are worked through roughly
  easiest → hardest. When you finish an item, move it out of `# Added` into an
  appropriate `# Done` subsection (or delete it if it was pure housekeeping
  with nothing worth recording).
- After any player-facing change, add a changelog line — see the section right
  below. Purely internal fixes (stale asset paths, refactors, dev-log cleanup)
  don't need one.
- Run `cargo check -p bevy-trickshot` (or `-p server` / `-p shared`) after
  editing, and `cargo test -p shared` if you touch ballistics/scoring/protocol
  code — that crate has the test coverage. That's the extent of verification
  to do yourself, though — see the next point.
- Never launch the client (`cargo run`) or take a screenshot to check that a
  change works, unless explicitly asked to. The user tests visually
  themselves, faster than any round trip through a screenshot would be — a
  passing `cargo check` is the bar to hit before handing a change back.
  Note the gap this leaves: Bevy validates query/system-param conflicts
  (`error[B0001]`, e.g. two `&mut Visibility` queries that aren't proven
  disjoint via `With`/`Without`) at schedule-build time, i.e. when the app
  actually starts — `cargo check` does not catch these. When adding or
  editing a query, manually re-check disjointness against every other query
  touching the same component in the same system, and say plainly that this
  particular class of bug can't be fully ruled out without the user running it.
- When working through `client/notes/todo.md`, do **one item at a time**:
  finish it, update the todo list and changelog, then stop and let the user
  test it before starting the next one. Don't chain multiple todo items into
  one uninterrupted pass.
- Never add a system to the client's `Startup` schedule — it blacks out the
  in-game 3D view. Use `PostStartup` instead.

## Changelog / "What's New" panel

The client shows a "WHAT'S NEW" panel on the main menu (`client/src/lobby_ui.rs`,
`build_browser`), backed by `client/src/changelog.rs`'s `ENTRIES` list. This is
the only place players see what changed — the client self-updates silently
(`client/src/updater.rs`) with no other release notification.

**Whenever you finish a player-facing feature or bug fix, add a line to
`ENTRIES` in `client/src/changelog.rs`** under the current in-progress
version (newest version first, newest line first within a version). If you
don't, players never find out it happened.

Bump the version in both places together when starting a new round of
changes destined for the next release:
- `Cargo.toml` (workspace root) `[workspace.package] version`
- the version key at the top of `client/src/changelog.rs`'s `ENTRIES`

The version is just a plain string bump (e.g. `0.4.3` → `0.4.4`) ahead of
actually tagging the release — tagging (`git tag vX.Y.Z && git push --tags`)
and the GitHub Actions release build are a separate, later step the user
triggers themselves.
