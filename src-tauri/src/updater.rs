//! GitHub Releases self-update (custom; no code signing required).
//!
//! The official tauri-updater needs signed artifacts and an app signing
//! identity. This project ships unsigned dev-reality builds (KI-004 works
//! around the missing identity), so the updater here is deliberately simple:
//!   - `check_for_update`: asks the GitHub API for the latest release and
//!     compares `tag_name` against `CARGO_PKG_VERSION` (curl + serde_json;
//!     no new HTTP deps).
//!   - `download_and_install_update`: downloads the `.app.tar.gz` asset,
//!     extracts it, swaps it in for the running bundle, relaunches via
//!     `open -n`, exits. Only allowed when running from a real `.app`
//!     bundle — in `tauri dev` the command reports `notInstalledApp` and
//!     the UI points at the release page instead.
//!
//! Stability rules (see docs/CONNECTION_REGRESSION_GUIDE.md §2.4): the
//! updater never touches the desktop/input/clipboard paths; replacing the
//! bundle on disk while running is safe because macOS keeps the running
//! binary's file handle — the swap only affects the on-disk name.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

const RELEASES_API: &str = "https://api.github.com/repos/zibochen6/jetson-tools/releases/latest";
const ASSET_SUFFIX: &str = ".app.tar.gz";

#[derive(Debug, Serialize)]
pub struct UpdateError {
    pub code: String,
    pub message: String,
}

impl UpdateError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub release_url: Option<String>,
    pub app_asset_url: Option<String>,
    /// True when running from an installed .app bundle (i.e. auto-install
    /// is possible); false in dev runs.
    pub is_bundled_app: bool,
}

fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// curl wrapper: GET + optional download. Returns (stdout, http_code).
fn curl_get(url: &str, out_file: Option<&Path>, max_secs: u32) -> Result<(String, u32), String> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "-L",
        "--max-time",
        &max_secs.to_string(),
        "--user-agent",
        "jetson-remote-updater",
        "-w",
        "\n%{http_code}",
    ]);
    if let Some(f) = out_file {
        cmd.arg("-o").arg(f);
    }
    cmd.arg(url);
    let out = cmd.output().map_err(|e| format!("curl: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines();
    let body = lines.next().unwrap_or("").to_string();
    let code: u32 = lines.next().unwrap_or("0").trim().parse().unwrap_or(0);
    Ok((body, code))
}

/// Compare dotted versions, e.g. `0.2.0` vs `v0.2.1`; returns true if b > a.
fn parse_version(s: &str) -> Vec<u64> {
    s.trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .split('.')
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect()
}

fn version_is_newer(current: &str, candidate: &str) -> bool {
    let a = parse_version(current);
    let b = parse_version(candidate);
    for i in 0..a.len().max(b.len()) {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        if bv > av {
            return true;
        }
        if av > bv {
            return false;
        }
    }
    false
}

fn running_bundle() -> Option<PathBuf> {
	let exe = std::env::current_exe().ok()?;
	// Walk up from …/Foo.app/Contents/MacOS/<bin> looking for the .app dir.
	let mut dir = exe.parent()?.to_path_buf();
	for _ in 0..4 {
		if dir.extension().map(|e| e == "app").unwrap_or(false) {
			return Some(dir);
		}
		dir = dir.parent()?.to_path_buf();
	}
	None
}

/// Check GitHub Releases for a newer version.
#[tauri::command]
pub async fn check_for_update() -> Result<UpdateCheckResult, UpdateError> {
    let current = current_version();
    let is_bundled = running_bundle().is_some();

    let (body, code) = tauri::async_runtime::spawn_blocking(move || curl_get(RELEASES_API, None, 30))
        .await
        .map_err(|e| UpdateError::new("network", e.to_string()))?
        .map_err(|e| UpdateError::new("network", e))?;

    if code == 404 {
        // No releases yet — nothing to update to.
        return Ok(UpdateCheckResult {
            current_version: current,
            latest_version: None,
            update_available: false,
            release_url: None,
            app_asset_url: None,
            is_bundled_app: is_bundled,
        });
    }
    if code == 403 || code == 429 {
        return Err(UpdateError::new(
            "rateLimited",
            "GitHub API 访问频率受限，请稍后再试",
        ));
    }
    if code != 200 || body.trim().is_empty() {
        return Err(UpdateError::new(
            "network",
            format!("GitHub API 响应异常 (HTTP {code})"),
        ));
    }

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| UpdateError::new("parse", e.to_string()))?;
    let tag = json["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_owned();
    let html_url = json["html_url"].as_str().map(str::to_owned);
    let body_text = json["body"].as_str().map(str::to_owned);
    let _ = body_text;

    let assets = json["assets"].as_array().cloned().unwrap_or_default();
    let app_asset = assets
        .iter()
        .filter_map(|a| {
            let name = a["name"].as_str()?;
            let url = a["browser_download_url"].as_str()?;
            Some((name.to_lowercase(), url.to_string()))
        })
        .find(|(name, _)| name.ends_with(ASSET_SUFFIX))
        .map(|(_, url)| url);

    let update_available = !tag.is_empty() && version_is_newer(&current, &tag);

    Ok(UpdateCheckResult {
        current_version: current,
        latest_version: (!tag.is_empty()).then_some(tag),
        update_available,
        release_url: html_url,
        app_asset_url: app_asset,
        is_bundled_app: is_bundled,
    })
}

/// Download + swap in the new bundle + relaunch. Never returns on success
/// (the process exits); errors are typed for the frontend.
#[tauri::command]
pub async fn download_and_install_update(app: tauri::AppHandle, url: String) -> Result<(), UpdateError> {
    tauri::async_runtime::spawn_blocking(move || do_install(url))
        .await
        .map_err(|e| UpdateError::new("install", e.to_string()))??;
    // Not reached on success (do_install exits); keep the compiler happy:
    let _ = app;
    Ok(())
}

fn do_install(url: String) -> Result<(), UpdateError> {
    let bundle = running_bundle().ok_or_else(|| {
        UpdateError::new(
            "notInstalledApp",
            "当前是开发版，不支持自动安装；请从 GitHub Release 页下载安装新的 app",
        )
    })?;
    let bundle_name = bundle
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Jetson Remote.app")
        .to_owned();
    let parent = bundle
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| UpdateError::new("install", "无法确定应用所在目录"))?;

    let work = std::env::temp_dir().join("jetson-remote-update");
    let tar_path = work.join("update.tar.gz");
    let extract = work.join("extract");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&extract).map_err(|e| UpdateError::new("install", e.to_string()))?;

    // 1. Download
    let (_, code) = curl_get(&url, Some(&tar_path), 600)
        .map_err(|e| UpdateError::new("download", format!("下载失败: {e}")))?;
    if code != 200 {
        return Err(UpdateError::new("download", format!("下载失败 (HTTP {code})")));
    }
    let meta = std::fs::metadata(&tar_path)
        .map_err(|e| UpdateError::new("download", format!("下载校验失败: {e}")))?;
    if meta.len() < 1024 * 1024 {
        return Err(UpdateError::new(
            "download",
            "下载内容异常（文件过小），请检查网络后重试",
        ));
    }

    // 2. Extract
    let tar_status = Command::new("tar")
        .args(["-xzf", tar_path.to_str().unwrap_or(""), "-C", extract.to_str().unwrap_or("")])
        .status()
        .map_err(|e| UpdateError::new("install", format!("解压失败: {e}")))?;
    if !tar_status.success() {
        return Err(UpdateError::new("install", "解压失败（tar 退出码非零）"));
    }

    // 3. Locate the new bundle
    let new_bundle = std::fs::read_dir(&extract)
        .map_err(|e| UpdateError::new("install", e.to_string()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "app").unwrap_or(false))
        .ok_or_else(|| UpdateError::new("install", "更新包中没有找到 .app"))?;
    let binary = new_bundle.join("Contents/MacOS/jetson-remote");
    if !binary.exists() {
        return Err(UpdateError::new("install", "更新包内容不完整（缺少可执行文件）"));
    }

    // 4. Swap: current → backup, new → current. Roll back on failure.
    let backup = parent.join(format!("{bundle_name}.old-{}", chrono_free()));
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::rename(&bundle, &backup)
        .map_err(|e| UpdateError::new("permission", format!("无法替换 {bundle_name}（{e}）。\n若安装在 /Applications，请将 app 移到 ~/Applications 或授予写入权限后重试")))?;
    if let Err(e) = std::fs::rename(&new_bundle, &bundle) {
        let _ = std::fs::rename(&backup, &bundle); // rollback
        return Err(UpdateError::new("install", format!("替换失败（已回滚）: {e}")));
    }
    let _ = std::fs::remove_dir_all(&backup);

    // 5. Relaunch and exit.
    let _ = Command::new("open").args(["-n", bundle.to_str().unwrap_or("")]).spawn();
    std::process::exit(0);
}

fn chrono_free() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{now}")
}

#[cfg(test)]
mod tests {
    use super::version_is_newer;

    #[test]
    fn version_compare() {
        assert!(version_is_newer("0.1.0", "0.2.0"));
        assert!(!version_is_newer("0.2.0", "0.2.0")); // equal -> not newer
        assert!(version_is_newer("0.2.0", "0.10.0"));
        assert!(version_is_newer("0.2.0", "0.2.1"));
        assert!(!version_is_newer("0.2.1", "0.2.0"));
        // prerelease suffix is stripped for comparison; a same-core rc is not newer
        assert!(!version_is_newer("1.0.0", "1.0.0-rc1"));
        assert!(version_is_newer("1.0.0", "1.0.1"));
    }
}