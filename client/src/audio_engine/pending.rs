//! Whether the sounds asked for so far have finished loading — what a client
//! waits on, along with the map, before it reports ready for a lobby game
//! (`game_start::send_assets_ready`), so nobody starts a match with clips
//! still loading (they'd play late, whenever they got in).
//!
//! Sounds are requested from all over the client, so there's no one list of
//! them; instead every clip file under `assets/audio/` (bar `unused/`) is
//! listed once, and the asset server asked which of those it has a handle
//! for. One nothing has asked for yet (a sound loaded on demand) isn't
//! waited on.

use std::path::Path;

use bevy::asset::io::file::FileAssetReader;
use bevy::asset::LoadState;
use bevy::prelude::*;

use super::{AudioSource, AUDIO_EXTENSIONS};

/// The clips still to check on, as asset paths (`audio/…`) — listed off disk
/// the first time [`Self::all_loaded`] is asked, and dropped as each turns
/// out loaded (or failed, or never asked for). Only ever shrinks: once
/// they're all in, they stay in for the rest of the launch.
#[derive(Resource, Default)]
pub(crate) struct PendingSounds(Option<Vec<String>>);

impl PendingSounds {
    /// Whether every clip that's been asked for has loaded (or failed to — a
    /// broken file won't ever turn up, and holding the party for it wouldn't
    /// help).
    pub(crate) fn all_loaded(&mut self, asset_server: &AssetServer) -> bool {
        let paths = self.0.get_or_insert_with(list_clips);
        paths.retain(|path| {
            asset_server
                .get_handle::<AudioSource>(path.as_str())
                .is_some_and(|h| matches!(asset_server.load_state(&h), LoadState::NotLoaded | LoadState::Loading))
        });
        paths.is_empty()
    }
}

fn list_clips() -> Vec<String> {
    let root = FileAssetReader::get_base_path().join("assets");
    let mut out = Vec::new();
    walk(&root, &root.join("audio"), &mut out);
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "unused") {
                continue;
            }
            walk(root, &path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        {
            if let Ok(rel) = path.strip_prefix(root) {
                // (Asset paths always use `/`.)
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}
