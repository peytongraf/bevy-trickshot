# bevy-trickshot (client)

## How to install and play (no coding needed)

You don't need to build anything or know how to program. Download a zip, unzip
it, and run the game. It updates itself after that.

### Windows

1. Open the **[Releases page](https://github.com/peytongraf/bevy-trickshot/releases)**
   and, under the newest release, download **`bevy-trickshot-windows.zip`**.
2. Right-click the downloaded file → **Extract All…**, and choose a folder to keep
   it in (for example, a new `bevy-trickshot` folder on your Desktop).
3. Open that folder and double-click **`bevy-trickshot.exe`**.
   - The first time, Windows may pop up a blue **“Windows protected your PC”**
     box. Click **More info**, then **Run anyway**. This only means the game
     isn't signed by a big company; it's safe.
4. The first launch downloads the artwork and sounds (about 90 MB), so give it a
   minute. Later launches are quick.

### Linux

1. From the **[Releases page](https://github.com/peytongraf/bevy-trickshot/releases)**,
   download **`bevy-trickshot-linux.zip`**.
2. Extract it, open a terminal in that folder, and run `./bevy-trickshot`.
   - If it says *permission denied*, run `chmod +x bevy-trickshot` once, then try
     again.

### After that

Just open the game the way you did before. When a new version is out, it updates
itself on startup (you'll see a brief “updating…” line) and then opens. You need
an internet connection for the update and to play with other people.

### Playing

- On first launch, type a **username** and confirm it.
- To play together, one person clicks **CREATE LOBBY**; everyone else clicks that
  lobby in the list to join. Once everyone is in, whoever made the lobby clicks
  **START GAME**.
- **W A S D** move, **mouse** looks, hold **right mouse** to aim through the
  scope, **left mouse** shoots, **Space** jumps. Press **Esc** for settings.
- **Can't connect to a friend's game?** Ask whoever is running the server — they
  may need to switch on an extra network option on their side.

---

This is the game client, one crate in the `bevy-trickshot/` workspace (see the
repo-root `README.md` for the server and the shared protocol/ballistics crate).
Run it with `cargo run --bin bevy-trickshot` from the repo root, or `cargo run`
from this directory. `cargo run` always points at a local dev server and never
triggers the self-updater (`src/updater.rs`) — see the repo-root `README.md`
for both the local dev workflow and the release/distribution flow.

**Settings menu** (`Esc`): username, mouse sensitivity, FOV, rebindable keys, and
a Debug Mode toggle (shows the egui tuning panels). Settings persist to
`~/.config/bevy-trickshot/settings.json` (`%APPDATA%\bevy-trickshot\` on Windows).
First launch with no username shows a username prompt. See `settings.rs`,
`keybinds.rs`, `menu.rs`.

## Controls (defaults, rebindable in the settings menu)

| Input                 | Action                              |
|-----------------------|--------------------------------------|
| `W` `A` `S` `D`       | Move                                 |
| Mouse                 | Look around                          |
| `Space`               | Jump                                 |
| Sprint bind (hold)    | Sprint                               |
| Crouch / Slide bind   | Crouch, or slide while sprinting     |
| left mouse            | Fire                                  |
| right mouse (hold)    | Aim down sight (scope)               |
| Reload bind           | Reload                               |
| Throwing Knife bind   | Throw knife                          |
| `Esc`                 | Settings / release the cursor        |

See `client/src/keybinds.rs` for the full, current default bindings — this
table only covers the common ones.

For the current architecture (killcam, weapons, avatars, lobby/net, HUD, etc.)
read the module-level doc comments in `client/src/` rather than this file; it's
easy for prose docs to drift as those systems change, but the `//!` comments at
the top of each module are kept next to the code they describe. `CLAUDE.md` at
the repo root also describes the project's goals and conventions.
