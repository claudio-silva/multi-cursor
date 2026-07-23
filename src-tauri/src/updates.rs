//! Check GitHub Releases for a newer Multi Cursor version.

use serde::Serialize;
use tauri::AppHandle;

use crate::about::REPO_URL;

const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/claudio-silva/multi-cursor/releases/latest";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub update_available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
    pub message: String,
}

#[derive(Debug, serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
}

fn normalize_version(raw: &str) -> String {
    raw.trim().trim_start_matches(['v', 'V']).to_string()
}

fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let s = normalize_version(raw);
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn is_newer(latest: &str, current: &str) -> Result<bool, String> {
    let latest_v = parse_semver(latest)
        .ok_or_else(|| format!("Could not parse latest version \"{latest}\""))?;
    let current_v = parse_semver(current)
        .ok_or_else(|| format!("Could not parse current version \"{current}\""))?;
    Ok(latest_v > current_v)
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateCheckResult, String> {
    let current_version = app.package_info().version.to_string();

    let client = reqwest::Client::builder()
        .user_agent(format!("multi-cursor/{current_version}"))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let response = client
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Failed to reach GitHub: {e}"))?;

    let status = response.status();
    if status.as_u16() == 404 {
        return Ok(UpdateCheckResult {
            update_available: false,
            current_version: current_version.clone(),
            latest_version: current_version.clone(),
            release_url: format!("{REPO_URL}/releases"),
            message: format!("You're up to date (version {current_version})."),
        });
    }
    if !status.is_success() {
        return Err(format!("GitHub returned HTTP {status}"));
    }

    let release: GithubRelease = response
        .json()
        .await
        .map_err(|e| format!("Invalid GitHub response: {e}"))?;

    let latest_version = normalize_version(&release.tag_name);
    let update_available = is_newer(&latest_version, &current_version)?;

    let message = if update_available {
        format!("Version {latest_version} is available.")
    } else {
        format!("You're up to date (version {current_version}).")
    };

    Ok(UpdateCheckResult {
        update_available,
        current_version,
        latest_version,
        release_url: release.html_url,
        message,
    })
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("URL is required".to_string());
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only http(s) URLs can be opened".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .status()
            .map_err(|e| format!("Failed to open URL: {e}"))?;
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err("Opening URLs is only supported on macOS".to_string())
    }
}
