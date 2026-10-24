//! The self-update check behind the launch update screen
//! (`update_screen`), run on a background thread so the window stays live.
//!
//! Flow (see `.github/workflows/release.yml` for the other half):
//!
//! 1. Every `git tag vX.Y.Z && git push --tags` makes GitHub Actions build the
//!    client for Linux and Windows and publish a **Release** containing:
//!      * `bevy-trickshot-linux.zip`   — just the executable
//!      * `bevy-trickshot-windows.zip` — just the executable
//!      * `assets.zip`                 — the whole `assets/` folder
//!      * `manifest.json`              — `{ "version", "assets_hash" }`
//! 2. On launch [`start`] fetches `manifest.json` from the *latest* release.
//!    If its `version` is newer than this build it downloads the matching
//!    executable zip and swaps this program's file in place. If
//!    `assets_hash` differs from what's on disk it refreshes `assets/` too.
//!    Either way the game then needs a restart ([`Progress::Ready`]), which
//!    the update screen asks for ([`restart`]).
//!
//! Nothing here fails open: no network, GitHub down, a bad download — it's
//! [`Progress::Failed`], and the update screen offers RETRY / QUIT rather
//! than letting an old build play. It's skipped entirely ([`enabled`]) under
//! `cargo run` (dev — always the latest code and assets) and when
//! `TRICKSHOT_SKIP_UPDATE` is set.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{env, fs};

/// `owner/repo` the updater reads Releases from. Override with `TRICKSHOT_REPO`.
const DEFAULT_REPO: &str = "peytongraf/bevy-trickshot";

/// Seconds to give up on looking a host up / connecting / its first byte.
/// (A download's body has no overall limit — the assets are big — but the
/// update screen shows it moving, and the window can always be closed.)
const RESOLVE_SECS: u64 = 10;
const CONNECT_SECS: u64 = 10;
const RESPONSE_SECS: u64 = 20;
/// The whole manifest fetch, end to end.
const MANIFEST_SECS: u64 = 30;

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// Where the check's got to — written by its thread, read each frame by the
/// update screen.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Progress {
    Checking,
    /// Downloading `what` for version `to`: `done` of `total` bytes (`total`
    /// 0 when the server doesn't say).
    Downloading {
        what: &'static str,
        to: String,
        done: u64,
        total: u64,
    },
    /// Version `version` is on disk — restart into it.
    Ready { version: String },
    UpToDate,
    /// Why it couldn't check or finish, for the player.
    Failed(String),
}

pub(crate) type SharedProgress = Arc<Mutex<Progress>>;

/// Whether this build checks for updates at all: not under `cargo run`, not
/// with `TRICKSHOT_SKIP_UPDATE` set (a fresh restart into a just-installed
/// update), and only on a platform there's a release build for.
pub(crate) fn enabled() -> bool {
    env::var_os("CARGO").is_none()
        && env::var_os("TRICKSHOT_SKIP_UPDATE").is_none()
        && ReleaseTarget::current().is_some()
}

/// Start the check on its own thread; it reports into what's returned.
pub(crate) fn start() -> SharedProgress {
    let progress: SharedProgress = Arc::new(Mutex::new(Progress::Checking));
    let out = progress.clone();
    std::thread::spawn(move || {
        let report = |p: Progress| {
            if let Ok(mut slot) = out.lock() {
                *slot = p;
            }
        };
        let result = match ReleaseTarget::current() {
            Some(target) => try_update(target, &report),
            None => Ok(Progress::UpToDate),
        };
        match result {
            Ok(done) => report(done),
            Err(e) => {
                eprintln!("[update] failed: {e}");
                report(Progress::Failed(failure_reason(&*e)));
            }
        }
    });
    progress
}

/// A failure in words a player can read.
fn failure_reason(e: &(dyn std::error::Error + 'static)) -> String {
    let text = e.to_string();
    if text.contains("404") {
        "No release was found to check against.".into()
    } else if text.to_lowercase().contains("timeout") || text.to_lowercase().contains("timed out") {
        "The update server took too long to answer.".into()
    } else if text.contains("dns") || text.contains("resolve") || text.contains("onnect") {
        "Couldn't reach the update server. Check your internet connection.".into()
    } else {
        format!("Something went wrong: {text}")
    }
}

fn try_update(target: ReleaseTarget, report: &dyn Fn(Progress)) -> Fallible<Progress> {
    let repo = env::var("TRICKSHOT_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string());
    let base = format!("https://github.com/{repo}/releases/latest/download");

    let manifest: Manifest = {
        let raw = http_text(&format!("{base}/manifest.json"))?;
        serde_json::from_str(&raw)?
    };
    let to = manifest.version.trim().trim_start_matches('v').to_string();

    let mut changed = false;
    if is_newer(&manifest.version, current_version())? {
        eprintln!("[update] v{} → v{to} — downloading…", current_version());
        let zip = download_to_temp(&format!("{base}/{}", target.binary_zip), target.binary_zip, "GAME", &to, report)?;
        let new_exe = extract_binary(&zip, target.binary_name)?;
        self_replace::self_replace(&new_exe)?;
        let _ = fs::remove_file(&zip);
        let _ = fs::remove_file(&new_exe);
        changed = true;
    }

    let have = fs::read_to_string(asset_stamp_path()).unwrap_or_default();
    if have.trim() != manifest.assets_hash && !manifest.assets_hash.is_empty() {
        eprintln!("[update] refreshing assets…");
        let zip = download_to_temp(&format!("{base}/assets.zip"), "assets.zip", "GAME FILES", &to, report)?;
        extract_all(&zip, Path::new("."))?;
        let _ = fs::remove_file(&zip);
        fs::write(asset_stamp_path(), &manifest.assets_hash)?;
        changed = true;
    }

    Ok(if changed { Progress::Ready { version: to } } else { Progress::UpToDate })
}

#[derive(serde::Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    assets_hash: String,
}

struct ReleaseTarget {
    /// Release asset name, stable so `releases/latest/download/<name>` resolves.
    binary_zip: &'static str,
    /// The executable's name inside that zip.
    binary_name: &'static str,
}

impl ReleaseTarget {
    fn current() -> Option<Self> {
        match (env::consts::OS, env::consts::ARCH) {
            ("linux", "x86_64") => Some(Self {
                binary_zip: "bevy-trickshot-linux.zip",
                binary_name: "bevy-trickshot",
            }),
            ("windows", "x86_64") => Some(Self {
                binary_zip: "bevy-trickshot-windows.zip",
                binary_name: "bevy-trickshot.exe",
            }),
            _ => None,
        }
    }
}

pub(crate) fn current_version() -> &'static str {
    shared::GAME_VERSION
}

fn is_newer(remote: &str, local: &str) -> Fallible<bool> {
    Ok(parse_version(remote)? > parse_version(local)?)
}

fn parse_version(s: &str) -> Result<semver::Version, semver::Error> {
    semver::Version::parse(s.trim().trim_start_matches('v'))
}

/// Whether version `a` is newer than `b` (`false` if either doesn't parse).
pub(crate) fn newer_than(a: &str, b: &str) -> bool {
    is_newer(a, b).unwrap_or(false)
}

/// Make the working directory the one that should hold `assets/`, so the
/// asset stamp and a download resolve no matter where the game was launched
/// from. Call first thing in `main()`.
///
/// * `./assets` already here (e.g. `cargo run`, or launched from the game folder)
///   → leave the CWD alone.
/// * otherwise, if there's an `assets/` next to the executable → go there.
/// * otherwise (fresh install, binary only) → go to the executable's directory so
///   the first-run asset download lands beside it.
pub(crate) fn point_cwd_at_assets() {
    if Path::new("assets").is_dir() {
        return;
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = env::set_current_dir(dir);
        }
    }
}

fn asset_stamp_path() -> PathBuf {
    Path::new("assets").join(".version")
}

/// Start a fresh copy of the game; the caller then exits this one. With
/// `check_again` the new one checks for updates on launch (the server said
/// there's a newer version); without, it skips the check (it's just been
/// updated, so it's current).
pub(crate) fn restart(check_again: bool) -> io::Result<()> {
    let exe = env::current_exe()?;
    let args: Vec<String> = env::args().skip(1).collect();
    let mut cmd = Command::new(exe);
    cmd.args(&args);
    if check_again {
        cmd.env_remove("TRICKSHOT_SKIP_UPDATE");
    } else {
        cmd.env("TRICKSHOT_SKIP_UPDATE", "1");
    }
    cmd.spawn().map(|_| ())
}

// ---- tiny HTTP + zip helpers -------------------------------------------------

fn agent(global: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_resolve(Some(Duration::from_secs(RESOLVE_SECS)))
        .timeout_connect(Some(Duration::from_secs(CONNECT_SECS)))
        .timeout_recv_response(Some(Duration::from_secs(RESPONSE_SECS)))
        .timeout_global(global)
        .build()
        .into()
}

fn http_text(url: &str) -> Fallible<String> {
    let mut resp = agent(Some(Duration::from_secs(MANIFEST_SECS))).get(url).call()?;
    Ok(resp.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string()?)
}

/// Download `url` to a temp file named after `name`, reporting progress as
/// `what` (for version `to`).
fn download_to_temp(url: &str, name: &str, what: &'static str, to: &str, report: &dyn Fn(Progress)) -> Fallible<PathBuf> {
    let mut resp = agent(None).get(url).call()?;
    let total: u64 = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let progress = |done: u64| {
        report(Progress::Downloading {
            what,
            to: to.to_string(),
            done,
            total,
        })
    };
    progress(0);

    let dest = env::temp_dir().join(format!("trickshot-{name}"));
    let mut out = fs::File::create(&dest)?;
    // No size cap: `assets.zip` only grows, and a fixed limit silently left
    // fresh installs without assets once it was crossed.
    let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();

    let mut buf = [0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut last_report = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if done - last_report >= 256 * 1024 {
            progress(done);
            last_report = done;
        }
    }
    progress(done);
    out.flush()?;
    if total > 0 && done != total {
        let _ = fs::remove_file(&dest);
        return Err(format!("{name} download incomplete ({done} of {total} bytes)").into());
    }
    Ok(dest)
}

/// Extract the whole archive under `dest`.
fn extract_all(zip_path: &Path, dest: &Path) -> Fallible<()> {
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path)?)?;
    archive.extract(dest)?;
    Ok(())
}

/// Extract just `wanted` from the archive to a temp file and return its path.
fn extract_binary(zip_path: &Path, wanted: &str) -> Fallible<PathBuf> {
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path)?)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let is_match = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|f| f == wanted))
            .unwrap_or(false);
        if !is_match {
            continue;
        }
        let out_path = env::temp_dir().join(format!("trickshot-new-{wanted}"));
        let mut out = fs::File::create(&out_path)?;
        io::copy(&mut entry, &mut out)?;
        out.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out_path, fs::Permissions::from_mode(0o755))?;
        }
        return Ok(out_path);
    }
    Err(format!("`{wanted}` not found inside {}", zip_path.display()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_with_or_without_a_v() {
        assert!(newer_than("0.4.8", "0.4.7"));
        assert!(newer_than("v0.5.0", "0.4.9"));
        assert!(!newer_than("0.4.7", "0.4.7"));
        assert!(!newer_than("0.4.6", "v0.4.7"));
        assert!(!newer_than("garbage", "0.4.7"));
    }
}
