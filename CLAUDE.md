# bevy-trickshot

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
