//! In-app updates from the GitHub Releases page.
//!
//! The app asks GitHub for the latest release, compares its version with its
//! own, and on request downloads the file for this computer and swaps it in:
//! - **Windows**: the new `.exe` replaces the running one (the old one is
//!   renamed out of the way first, which Windows allows), then it restarts.
//! - **Mac**: the new `File Tracker.app` is unpacked next to the current one
//!   and the two are swapped, then the new one is opened. The old copy is
//!   removed the next time the app starts.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "tomklane/file-tracker";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub page: String,
    pub notes: String,
    pub asset_url: Option<String>,
    pub asset_size: u64,
}

/// This app's version. (Test builds can pretend to be older with FT_PRETEND_VERSION.)
pub fn current_version() -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("FT_PRETEND_VERSION") {
        return v;
    }
    CURRENT.to_string()
}

/// The release file built for this kind of computer.
pub fn asset_name() -> Option<String> {
    #[cfg(debug_assertions)]
    if let Ok(a) = std::env::var("FT_PRETEND_ASSET") {
        return Some(a);
    }
    if cfg!(target_os = "windows") {
        Some("File-Tracker-Windows.exe".into())
    } else if cfg!(target_os = "macos") {
        Some("File-Tracker-Mac.zip".into())
    } else {
        None
    }
}

/// Can this copy of the app update itself, or only point to the download page?
pub fn can_self_update() -> bool {
    asset_name().is_some()
}

fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder()
        // trust the same certificates as the rest of the computer (works behind office proxies)
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .user_agent(format!("FileTracker/{}", current_version()))
        .timeout_connect(Some(Duration::from_secs(15)))
        // redirects are followed by `get` below: following them automatically
        // fails on some networks when GitHub's redirect reply is chunked
        .max_redirects(0)
        .build()
        .into()
}

/// GET a URL, following up to 5 redirects.
fn get(url: &str, timeout: Duration, headers: &[(&str, &str)]) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let agent = agent();
    let mut url = url.to_string();
    for _ in 0..5 {
        let mut req = agent.get(&url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let mut resp = req.config().timeout_global(Some(timeout)).build().call()?;
        if resp.status().is_redirection() {
            if let Some(loc) = resp.headers().get("location").and_then(|v| v.to_str().ok()) {
                let next = if loc.starts_with("http") {
                    loc.to_string()
                } else {
                    // relative redirect
                    let base = url::Url::parse(&url).map_err(|_| ureq::Error::BadUri(url.clone()))?;
                    base.join(loc).map_err(|_| ureq::Error::BadUri(loc.to_string()))?.to_string()
                };
                let _ = resp.body_mut().read_to_string(); // finish the redirect reply
                url = next;
                continue;
            }
        }
        return Ok(resp);
    }
    Err(ureq::Error::TooManyRedirects)
}

fn parse_version(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+'])
        .take(3)
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0))
        .collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse_version(latest), parse_version(current));
    a.resize(3, 0);
    b.resize(3, 0);
    a > b
}

/// Read the latest release from GitHub's JSON answer.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|_| "GitHub sent an unexpected answer.".to_string())?;
    let tag = v["tag_name"].as_str().ok_or("No releases have been published yet.")?;
    let asset = asset_name().and_then(|name| {
        v["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(name.as_str())).cloned()
    });
    Ok(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        notes: v["body"].as_str().unwrap_or("").to_string(),
        asset_url: asset.as_ref().and_then(|a| a["browser_download_url"].as_str()).map(String::from),
        asset_size: asset.as_ref().and_then(|a| a["size"].as_u64()).unwrap_or(0),
    })
}

pub fn check_latest() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut resp = get(&url, Duration::from_secs(20), &[("Accept", "application/vnd.github+json")]).map_err(|e| match e {
            ureq::Error::StatusCode(404) => "No releases found (is the repository private?).".to_string(),
            ureq::Error::StatusCode(403) | ureq::Error::StatusCode(429) => {
                "GitHub is limiting requests right now. Try again in a while.".to_string()
            }
            other => format!("Couldn't reach GitHub ({other})."),
        })?;
    let body = resp.body_mut().read_to_string().map_err(|e| format!("Couldn't read GitHub's answer ({e})."))?;
    parse_release(&body)
}

/// Where downloads and unpacking happen: next to the app when possible, so the
/// final swap is a quick rename on the same drive.
fn work_dir() -> PathBuf {
    let base = std::env::current_exe()
        .ok()
        .and_then(|exe| app_bundle(&exe).or_else(|| exe.parent().map(Path::to_path_buf)))
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .filter(|p| is_writable(p))
        .unwrap_or_else(std::env::temp_dir);
    base.join(".file-tracker-update")
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".ft-write-test-{}", std::process::id()));
    let ok = fs::write(&probe, b"x").is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

/// Download the release file, reporting (bytes so far, total).
pub fn download(rel: &Release, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    let url = rel.asset_url.as_deref().ok_or("This release has no download for this computer.")?;
    let dir = work_dir();
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make a download folder ({e})."))?;
    let dest = dir.join(asset_name().unwrap_or_else(|| "download".into()));
    let mut resp = get(url, Duration::from_secs(900), &[]).map_err(|e| format!("Download failed ({e})."))?;
    let total = resp.body().content_length().unwrap_or(rel.asset_size);
    let mut reader = resp.body_mut().with_config().limit(500 * 1024 * 1024).reader();
    let mut out = fs::File::create(&dest).map_err(|e| format!("Couldn't save the download ({e})."))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Download interrupted ({e})."))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| format!("Couldn't save the download ({e})."))?;
        got += n as u64;
        progress(got, total);
    }
    out.flush().map_err(|e| e.to_string())?;
    if total > 0 && got != total {
        return Err(format!("Download incomplete ({got} of {total} bytes). Please try again."));
    }
    Ok(dest)
}

/// The `.app` folder this program runs from, if any.
pub fn app_bundle(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().find(|p| p.extension().map(|e| e == "app").unwrap_or(false)).map(Path::to_path_buf)
}

fn old_copy_of(bundle: &Path) -> PathBuf {
    let name = bundle.file_name().unwrap_or_default().to_string_lossy().to_string();
    bundle.with_file_name(format!("{name}.old"))
}

/// Unpack a zip into `dir`, keeping file permissions (so the app stays runnable).
pub fn unzip(zip_path: &Path, dir: &Path) -> Result<(), String> {
    let f = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("The download isn't a valid zip ({e})."))?;
    for i in 0..z.len() {
        let mut entry = z.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else { continue }; // ignore unsafe paths
        let out = dir.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(p) = out.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut w = fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut w).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o7777));
        }
    }
    Ok(())
}

/// Swap an app bundle for the one inside `zip_path`. Returns the path to open.
/// On any failure the original app is put back.
pub fn swap_bundle(zip_path: &Path, bundle: &Path) -> Result<PathBuf, String> {
    let parent = bundle.parent().ok_or("Can't find the app's folder.")?;
    let stage = parent.join(".file-tracker-new");
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage).map_err(|e| no_permission(parent, e))?;
    let result = (|| {
        unzip(zip_path, &stage)?;
        let new_app = fs::read_dir(&stage)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().map(|e| e == "app").unwrap_or(false))
            .ok_or("The download doesn't contain the app.")?;
        let old = old_copy_of(bundle);
        let _ = fs::remove_dir_all(&old);
        fs::rename(bundle, &old).map_err(|e| no_permission(parent, e))?;
        if let Err(e) = fs::rename(&new_app, bundle) {
            let _ = fs::rename(&old, bundle); // put the original back
            return Err(format!("Couldn't put the new version in place ({e})."));
        }
        Ok(bundle.to_path_buf())
    })();
    let _ = fs::remove_dir_all(&stage);
    result
}

fn no_permission(dir: &Path, e: std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        format!(
            "File Tracker isn't allowed to change “{}”. Download the new version from the release page instead.",
            dir.display()
        )
    } else {
        e.to_string()
    }
}

/// Put the downloaded update in place. Returns what to launch afterwards.
pub fn install(downloaded: &Path) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if cfg!(target_os = "macos") {
        if let Some(bundle) = app_bundle(&exe) {
            return swap_bundle(downloaded, &bundle);
        }
        // a bare binary (e.g. built by hand): take the program out of the zip
        let stage = downloaded.with_extension("unzipped");
        let _ = fs::remove_dir_all(&stage);
        unzip(downloaded, &stage)?;
        let bin = walk_find(&stage, "File Tracker").ok_or("The download doesn't contain the app.")?;
        self_replace::self_replace(&bin).map_err(|e| no_permission(exe.parent().unwrap_or(&exe), e))?;
        let _ = fs::remove_dir_all(&stage);
        return Ok(exe);
    }
    // Windows (and anything else): replace the running program file
    self_replace::self_replace(downloaded).map_err(|e| no_permission(exe.parent().unwrap_or(&exe), e))?;
    Ok(exe)
}

fn walk_find(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = walk_find(&p, name) {
                return Some(f);
            }
        } else if p.file_name().map(|n| n == name).unwrap_or(false) && p.parent().map(|q| q.ends_with("MacOS")).unwrap_or(false) {
            return Some(p);
        }
    }
    None
}

/// Start the new version and quit this one.
pub fn relaunch(target: &Path) -> ! {
    let _ = fs::remove_dir_all(work_dir());
    #[cfg(target_os = "macos")]
    {
        if target.extension().map(|e| e == "app").unwrap_or(false) {
            let _ = std::process::Command::new("open").arg("-n").arg(target).spawn();
            std::process::exit(0);
        }
    }
    let _ = std::process::Command::new(target).spawn();
    std::process::exit(0);
}

/// Remove what a previous update left behind.
pub fn cleanup_after_update() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bundle) = app_bundle(&exe) {
            let _ = fs::remove_dir_all(old_copy_of(&bundle));
            if let Some(parent) = bundle.parent() {
                let _ = fs::remove_dir_all(parent.join(".file-tracker-new"));
            }
        }
    }
    let _ = fs::remove_dir_all(work_dir());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("v1.3.0", "1.2.0"));
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(is_newer("2.0", "1.99.99"));
        assert!(!is_newer("v1.2.0", "1.2.0"));
        assert!(!is_newer("1.1.9", "1.2.0"));
        assert!(!is_newer("garbage", "1.2.0"));
    }

    #[test]
    fn release_json() {
        let json = r#"{"tag_name":"v1.3.0","html_url":"https://example/r","body":"notes",
            "assets":[{"name":"File-Tracker-Mac.zip","size":10,"browser_download_url":"https://example/mac"},
                      {"name":"File-Tracker-Windows.exe","size":20,"browser_download_url":"https://example/win"}]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "1.3.0");
        assert_eq!(r.notes, "notes");
        // on Linux there's no self-update asset; on Mac/Windows the right one is chosen
        match asset_name().as_deref() {
            Some("File-Tracker-Mac.zip") => assert_eq!(r.asset_url.as_deref(), Some("https://example/mac")),
            Some(_) => assert_eq!(r.asset_url.as_deref(), Some("https://example/win")),
            None => assert!(r.asset_url.is_none()),
        }
        assert!(parse_release(r#"{"message":"Not Found"}"#).is_err());
    }

    /// Builds a fake installed app and a fake update zip, then swaps them.
    #[test]
    fn bundle_swap() {
        let base = std::env::temp_dir().join(format!("ft-swap-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let apps = base.join("Applications");
        let bundle = apps.join("File Tracker.app");
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(bundle.join("Contents/MacOS/File Tracker"), b"old version").unwrap();

        let zip_path = base.join("update.zip");
        {
            let mut z = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            z.add_directory("File Tracker.app/Contents/MacOS/", opts).unwrap();
            z.start_file("File Tracker.app/Contents/MacOS/File Tracker", opts).unwrap();
            z.write_all(b"new version").unwrap();
            z.start_file("../escape.txt", opts).unwrap(); // must be ignored
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let opened = swap_bundle(&zip_path, &bundle).unwrap();
        assert_eq!(opened, bundle);
        assert_eq!(fs::read(bundle.join("Contents/MacOS/File Tracker")).unwrap(), b"new version");
        assert_eq!(fs::read(old_copy_of(&bundle).join("Contents/MacOS/File Tracker")).unwrap(), b"old version");
        assert!(!apps.join(".file-tracker-new").exists());
        assert!(!base.join("escape.txt").exists() && !apps.join("escape.txt").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(bundle.join("Contents/MacOS/File Tracker")).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "the app must stay runnable");
        }
        // a broken download leaves the installed app untouched
        fs::write(&zip_path, b"not a zip").unwrap();
        assert!(swap_bundle(&zip_path, &bundle).is_err());
        assert_eq!(fs::read(bundle.join("Contents/MacOS/File Tracker")).unwrap(), b"new version");
        let _ = fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod live {
    /// Talks to the real GitHub release. Run with: cargo test live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_check_and_download() {
        let r = super::check_latest().expect("check");
        println!("latest {} (current {}) page {}", r.version, super::current_version(), r.page);
        assert!(!r.version.is_empty());
        // pretend to be on Windows: download the Windows asset to prove downloads work end to end
        let json_url = format!("https://github.com/{}/releases/download/v{}/File-Tracker-Windows.exe", super::REPO, r.version);
        let rel = super::Release { asset_url: Some(json_url), asset_size: 0, ..r };
        let last = std::cell::Cell::new(0u64);
        let p = super::download(&rel, &|got, total| {
            if got - last.get() > 4_000_000 || got == total {
                println!("  {got} / {total}");
                last.set(got);
            }
        })
        .expect("download");
        let len = std::fs::metadata(&p).unwrap().len();
        let head = std::fs::read(&p).unwrap()[..2].to_vec();
        println!("downloaded {} bytes to {}, starts with {:?}", len, p.display(), String::from_utf8_lossy(&head));
        assert_eq!(&head, b"MZ");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}

