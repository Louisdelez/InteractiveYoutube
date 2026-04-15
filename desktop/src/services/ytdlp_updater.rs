// Auto-updater for yt-dlp (desktop).
//
// Owns a user-local binary under $XDG_DATA_HOME/KoalaTV/bin/yt-dlp.
// Runs at launch: downloads if missing, else invokes `yt-dlp -U`. A background
// thread repeats the update every UPDATE_INTERVAL. The resolved path is
// injected into libmpv via `script-opts=ytdl_hook-ytdl_path=...` so the player
// always uses the freshest extractor.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

const DOWNLOAD_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp";
const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

static YTDLP_PATH: OnceLock<PathBuf> = OnceLock::new();

fn data_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("KoalaTV");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".local/share/KoalaTV")
}

pub fn binary_path() -> PathBuf {
    YTDLP_PATH
        .get_or_init(|| data_dir().join("bin/yt-dlp"))
        .clone()
}

fn set_executable(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    { let _ = path; }
    Ok(())
}

fn download_to(path: &std::path::Path) -> anyhow::Result<()> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(120))
        .build()?;
    let bytes = client.get(DOWNLOAD_URL).send()?.error_for_status()?.bytes()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &bytes)?;
    set_executable(&tmp)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn version_of(bin: &std::path::Path) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    if !out.status.success() { return None; }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn self_update(bin: &std::path::Path) {
    let before = version_of(bin);
    let out = Command::new(bin)
        .args(["-U", "--update-to", "stable"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let after = version_of(bin);
            if before != after {
                eprintln!(
                    "yt-dlp: updated {:?} -> {:?}",
                    before.as_deref(), after.as_deref()
                );
            } else {
                eprintln!("yt-dlp: up-to-date ({:?})", after.as_deref());
            }
        }
        Ok(o) => eprintln!(
            "yt-dlp: self-update failed ({}): {}",
            o.status,
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(err) => eprintln!("yt-dlp: self-update spawn error: {}", err),
    }
}

fn tick() {
    let path = binary_path();
    if !path.exists() {
        eprintln!("yt-dlp: bootstrapping binary at {}", path.display());
        if let Err(err) = download_to(&path) {
            eprintln!("yt-dlp: bootstrap failed: {}", err);
            return;
        }
    }
    self_update(&path);
}

/// Spawn a daemon thread that runs an immediate update then repeats every
/// UPDATE_INTERVAL. Safe to call once at startup.
pub fn spawn() {
    std::thread::Builder::new()
        .name("ytdlp-updater".into())
        .spawn(|| loop {
            tick();
            std::thread::sleep(UPDATE_INTERVAL);
        })
        .expect("spawn ytdlp-updater thread");
}
