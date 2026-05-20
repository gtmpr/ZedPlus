use anyhow::{Context, Result};
use std::io::Write as IoWrite;

pub const REPO: &str = "gtmpr/ZedPlus";
pub const RELEASES_URL: &str = "https://api.github.com/repos/gtmpr/ZedPlus/releases/latest";

pub struct ReleaseInfo {
    pub tag: String,
    pub version: String,
    pub asset_url: Option<String>,
    pub asset_name: Option<String>,
}

/// Query the GitHub releases API for the latest release.
pub async fn fetch_latest(client: &reqwest::Client) -> Result<ReleaseInfo> {
    let data: serde_json::Value = client
        .get(RELEASES_URL)
        .header("User-Agent", "zedplus-updater")
        .send()
        .await
        .context("network error checking for updates")?
        .json()
        .await
        .context("failed to parse release JSON")?;

    let tag = data["tag_name"].as_str().unwrap_or("").to_string();
    let version = tag.trim_start_matches('v').to_string();

    // Match the asset for the current platform
    let asset_suffix = platform_asset_suffix();
    let mut asset_url = None;
    let mut asset_name = None;
    if let Some(assets) = data["assets"].as_array() {
        for asset in assets {
            let name = asset["name"].as_str().unwrap_or("");
            if name.ends_with(asset_suffix) {
                asset_url = asset["browser_download_url"].as_str().map(String::from);
                asset_name = Some(name.to_string());
                break;
            }
        }
    }

    Ok(ReleaseInfo { tag, version, asset_url, asset_name })
}

/// Returns the expected filename suffix for the current platform's release asset.
fn platform_asset_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x86_64.zip"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-arm64.tar.gz"
        } else {
            "macos-x86_64.tar.gz"
        }
    } else {
        "linux-x86_64.tar.gz"
    }
}

/// Download the asset at `url` to a temp file and return the path.
async fn download_asset(client: &reqwest::Client, url: &str, name: &str) -> Result<std::path::PathBuf> {
    use futures::StreamExt;
    let resp = client
        .get(url)
        .header("User-Agent", "zedplus-updater")
        .send()
        .await
        .context("download failed")?;

    let total = resp.content_length();
    let mut stream = resp.bytes_stream();

    let tmp_path = std::env::temp_dir().join(name);
    let mut file = std::fs::File::create(&tmp_path).context("could not create temp file")?;

    let mut downloaded: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("stream error")?;
        file.write_all(&bytes)?;
        downloaded += bytes.len() as u64;
        if let Some(total) = total {
            let pct = downloaded * 100 / total;
            eprint!("\r  Downloading… {pct}%");
            let _ = std::io::stderr().flush();
        }
    }
    eprintln!();
    Ok(tmp_path)
}

/// Extract `zedplus` / `zedplus.exe` from a zip or tar.gz archive at `archive_path`.
fn extract_binary(archive_path: &std::path::Path) -> Result<std::path::PathBuf> {
    let bin_name = if cfg!(windows) { "zedplus.exe" } else { "zedplus" };
    let out_path = archive_path.with_file_name(bin_name);

    let ext = archive_path.to_string_lossy();
    if ext.ends_with(".zip") {
        extract_from_zip(archive_path, bin_name, &out_path)?;
    } else {
        extract_from_targz(archive_path, bin_name, &out_path)?;
    }
    Ok(out_path)
}

fn extract_from_zip(archive: &std::path::Path, bin_name: &str, out: &std::path::Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).context("invalid zip")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.name().ends_with(bin_name) {
            let mut out_file = std::fs::File::create(out)?;
            std::io::copy(&mut entry, &mut out_file)?;
            return Ok(());
        }
    }
    anyhow::bail!("binary not found in zip archive")
}

fn extract_from_targz(archive: &std::path::Path, bin_name: &str, out: &std::path::Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        if path.file_name().and_then(|n| n.to_str()) == Some(bin_name) {
            let mut out_file = std::fs::File::create(out)?;
            std::io::copy(&mut entry, &mut out_file)?;
            return Ok(());
        }
    }
    anyhow::bail!("binary not found in tar.gz archive")
}

/// Replace the current running binary with `new_binary`.
/// On Windows, can't replace a running exe — stage it as `zedplus_new.exe` next to current.
fn install_binary(new_binary: &std::path::Path) -> Result<std::path::PathBuf> {
    let current = std::env::current_exe().context("could not determine current binary path")?;

    #[cfg(windows)]
    {
        // Can't replace running exe on Windows; place next to current with _new suffix
        let staged = current.with_file_name("zedplus_new.exe");
        std::fs::copy(new_binary, &staged)?;
        return Ok(staged);
    }

    #[cfg(not(windows))]
    {
        // Set executable bit
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(new_binary)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(new_binary, perms)?;

        // Atomic rename (may fail cross-device; fall back to copy+rename)
        if std::fs::rename(new_binary, &current).is_err() {
            std::fs::copy(new_binary, &current)?;
        }
        return Ok(current);
    }
}

/// Full self-update flow. Returns the installed path.
pub async fn perform_update(client: &reqwest::Client, release: &ReleaseInfo) -> Result<std::path::PathBuf> {
    let asset_url = release.asset_url.as_deref()
        .ok_or_else(|| anyhow::anyhow!("no matching asset for this platform"))?;
    let asset_name = release.asset_name.as_deref().unwrap_or("zedplus-update");

    eprintln!("  Downloading {} …", asset_name);
    let archive = download_asset(client, asset_url, asset_name).await?;
    eprintln!("  Extracting binary…");
    let bin = extract_binary(&archive)?;
    eprintln!("  Installing…");
    let installed = install_binary(&bin)?;
    // Clean up
    let _ = std::fs::remove_file(&archive);
    let _ = std::fs::remove_file(&bin);
    Ok(installed)
}
