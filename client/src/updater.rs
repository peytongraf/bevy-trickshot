//! Launch-time self-update.
//!
//! Flow (see `.github/workflows/release.yml` for the other half):
//!
//! 1. Every `git tag vX.Y.Z && git push --tags` makes GitHub Actions build the
//!    client for Linux and Windows and publish a **Release** containing:
//!      * `bevy-trickshot-linux.zip`   — just the executable
//!      * `bevy-trickshot-windows.zip` — just the executable
//!      * `assets.zip`                 — the whole `assets/` folder
//!      * `manifest.json`              — `{ "version", "assets_hash" }`
//! 2. On launch [`bootstrap`] fetches `manifest.json` from the *latest* release.
//!    If its `version` is newer than this build it downloads the matching
//!    executable zip, swaps this program's file in place and relaunches. If
//!    `assets_hash` differs from what's on disk it refreshes `assets/` too.
//!
//! Everything here fails soft: no network, no releases yet, GitHub down — it logs
//! a line and lets the game start on the current version. It also no-ops entirely
//! under `cargo run` (dev) and when `TRICKSHOT_SKIP_UPDATE` is set.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

/// `owner/repo` the updater reads Releases from. Override with `TRICKSHOT_REPO`.
/// TODO: confirm this matches where you push — it's a guess from your git config.
const DEFAULT_REPO: &str = "peytongraf/bevy-trickshot";

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// Call once, first thing in `main()`. May replace this executable and exit the
/// process (a fresh copy is spawned in its place).
pub fn bootstrap() {
    point_cwd_at_assets();

    if env::var_os("CARGO").is_some() || env::var_os("TRICKSHOT_SKIP_UPDATE").is_some() {
        return;
    }
    let Some(target) = ReleaseTarget::current() else {
        return; // unsupported OS/arch — ship it as-is
    };

    match try_update(target) {
        Ok(Outcome::UpToDate) => {}
        Ok(Outcome::AssetsRefreshed) => {
            eprintln!("[update] assets refreshed; starting v{}", current_version());
        }
        Ok(Outcome::BinaryReplaced) => {
            eprintln!("[update] updated — restarting…");
            relaunch();
        }
        Err(e) => eprintln!("[update] skipped ({e}); starting v{}", current_version()),
    }
}

enum Outcome {
    UpToDate,
    AssetsRefreshed,
    BinaryReplaced,
}

fn try_update(target: ReleaseTarget) -> Fallible<Outcome> {
    let repo = env::var("TRICKSHOT_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string());
    let base = format!("https://github.com/{repo}/releases/latest/download");

    let manifest: Manifest = {
        let raw = http_text(&format!("{base}/manifest.json"))?;
        serde_json::from_str(&raw)?
    };

    let mut binary_replaced = false;
    if is_newer(&manifest.version, current_version())? {
        eprintln!(
            "[update] v{} → v{} — downloading…",
            current_version(),
            manifest.version
        );
        let zip = download_to_temp(&format!("{base}/{}", target.binary_zip), target.binary_zip)?;
        let new_exe = extract_binary(&zip, target.binary_name)?;
        self_replace::self_replace(&new_exe)?;
        let _ = fs::remove_file(&zip);
        let _ = fs::remove_file(&new_exe);
        binary_replaced = true;
    }

    let mut assets_refreshed = false;
    let have = fs::read_to_string(asset_stamp_path()).unwrap_or_default();
    if have.trim() != manifest.assets_hash && !manifest.assets_hash.is_empty() {
        eprintln!("[update] refreshing assets…");
        let zip = download_to_temp(&format!("{base}/assets.zip"), "assets.zip")?;
        extract_all(&zip, Path::new("."))?;
        let _ = fs::remove_file(&zip);
        fs::write(asset_stamp_path(), &manifest.assets_hash)?;
        assets_refreshed = true;
    }

    Ok(match (binary_replaced, assets_refreshed) {
        (true, _) => Outcome::BinaryReplaced,
        (false, true) => Outcome::AssetsRefreshed,
        (false, false) => Outcome::UpToDate,
    })
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

fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn is_newer(remote: &str, local: &str) -> Fallible<bool> {
    let parse = |s: &str| semver::Version::parse(s.trim().trim_start_matches('v'));
    Ok(parse(remote)? > parse(local)?)
}

/// Make the working directory the one that should hold `assets/`, so Bevy and the
/// asset stamp resolve no matter where the game was launched from:
///
/// * `./assets` already here (e.g. `cargo run`, or launched from the game folder)
///   → leave the CWD alone.
/// * otherwise, if there's an `assets/` next to the executable → go there.
/// * otherwise (fresh install, binary only) → go to the executable's directory so
///   the first-run asset download lands beside it.
fn point_cwd_at_assets() {
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

fn relaunch() -> ! {
    let exe = env::current_exe().expect("current_exe");
    let args: Vec<String> = env::args().skip(1).collect();
    let spawned = Command::new(exe)
        .args(&args)
        .env("TRICKSHOT_SKIP_UPDATE", "1")
        .spawn();
    match spawned {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            eprintln!("[update] could not relaunch ({e}); continuing on the new version anyway");
            // The in-memory code is still the old version; safest to just stop.
            std::process::exit(0);
        }
    }
}

// ---- tiny HTTP + zip helpers -------------------------------------------------

const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

fn http_text(url: &str) -> Fallible<String> {
    let mut resp = ureq::get(url).call()?;
    Ok(resp.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string()?)
}

fn download_to_temp(url: &str, name: &str) -> Fallible<PathBuf> {
    let mut resp = ureq::get(url).call()?;
    let total: u64 = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let dest = env::temp_dir().join(format!("trickshot-{name}"));
    let mut out = fs::File::create(&dest)?;
    let mut reader = resp.body_mut().with_config().limit(MAX_DOWNLOAD).reader();

    let mut buf = [0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut last_print = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if done - last_print >= 1024 * 1024 {
            print_progress(name, done, total);
            last_print = done;
        }
    }
    print_progress(name, done, total);
    eprintln!();
    out.flush()?;
    Ok(dest)
}

fn print_progress(name: &str, done: u64, total: u64) {
    let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
    if total > 0 {
        eprint!(
            "\r[update] {name}  {:>5.1}%  ({:.1}/{:.1} MB)   ",
            done as f64 / total as f64 * 100.0,
            mb(done),
            mb(total)
        );
    } else {
        eprint!("\r[update] {name}  {:.1} MB   ", mb(done));
    }
    let _ = io::stderr().flush();
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
