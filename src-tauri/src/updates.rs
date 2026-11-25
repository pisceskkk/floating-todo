use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const REPO_PATH: &str = "pisceskkk/floating-todo";
const RELEASES_URL: &str = "https://github.com/pisceskkk/floating-todo/releases";
const RELEASES_ATOM_URL: &str = "https://github.com/pisceskkk/floating-todo/releases.atom";
const RELEASES_LATEST_URL: &str = "https://github.com/pisceskkk/floating-todo/releases/latest";
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/pisceskkk/floating-todo/releases/download/";
const MANIFEST_FILENAME: &str = "update-manifest.json";
#[cfg(target_os = "macos")]
const EXPECTED_BUNDLE_ID: &str = "com.gaopengfei.floating-todo";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedAsset {
    name: String,
    download_url: String,
    sha256: Option<String>,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct UpdateManifest {
    #[serde(default)]
    platforms: HashMap<String, ManifestPlatform>,
}

#[derive(Debug, Deserialize)]
struct ManifestPlatform {
    url: String,
    sha256: Option<String>,
    size: Option<u64>,
    name: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    current_version: String,
    version: Option<String>,
    url: String,
    can_install: bool,
    asset_name: Option<String>,
    asset_size: Option<u64>,
    #[serde(skip)]
    download_url: Option<String>,
    #[serde(skip)]
    sha256: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    stage: String,
    downloaded: u64,
    total: Option<u64>,
    percent: Option<u8>,
    message: String,
}

#[derive(Default)]
struct UpdateCache {
    etag: Option<String>,
    last_status: Option<UpdateStatus>,
}

#[derive(Default)]
pub struct UpdateState {
    cache: tokio::sync::Mutex<UpdateCache>,
    installing: AtomicBool,
}

struct InstallGuard<'a>(&'a AtomicBool);
impl Drop for InstallGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn current_platform_key() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some("darwin-aarch64")
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Some("darwin-x86_64")
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Some("windows-x86_64")
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        Some("windows-aarch64")
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
    )))]
    {
        None
    }
}

fn is_valid_download_url(url: &str) -> bool {
    url.starts_with(RELEASE_DOWNLOAD_PREFIX) && !url.contains("..")
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn extract_tag_from_release_url(url: &str) -> Option<String> {
    let marker = "/releases/tag/";
    let idx = url.find(marker)?;
    let tail = &url[idx + marker.len()..];
    let raw = tail.split(['?', '#', '/']).next()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(percent_decode(raw))
}

fn parse_atom_releases(xml: &str) -> Result<Vec<Release>, String> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|_| "GitHub 返回了无法识别的更新信息".to_string())?;
    let mut releases = Vec::new();
    for entry in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "entry")
    {
        let mut tag = None;
        for child in entry.children().filter(|n| n.is_element()) {
            if child.tag_name().name() == "link" {
                if let Some(href) = child.attribute("href") {
                    if let Some(t) = extract_tag_from_release_url(href) {
                        tag = Some(t);
                        break;
                    }
                }
            }
        }
        if tag.is_none() {
            if let Some(id_text) = entry
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "id")
                .and_then(|n| n.text())
            {
                if let Some(last) = id_text.rsplit('/').next() {
                    let trimmed = last.trim();
                    if !trimmed.is_empty() {
                        tag = Some(trimmed.to_string());
                    }
                }
            }
        }
        if let Some(tag_name) = tag {
            let prerelease = Version::parse(tag_name.strip_prefix('v').unwrap_or(&tag_name))
                .map(|v| !v.pre.is_empty())
                .unwrap_or(false);
            releases.push(Release {
                tag_name,
                draft: false,
                prerelease,
            });
        }
    }
    Ok(releases)
}

fn select_release<'a>(current: &Version, releases: &'a [Release]) -> Option<(Version, &'a Release)> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| {
            let version =
                Version::parse(r.tag_name.strip_prefix('v').unwrap_or(&r.tag_name)).ok()?;
            if current.pre.is_empty() && (r.prerelease || !version.pre.is_empty()) {
                return None;
            }
            (version.cmp_precedence(current).is_gt()).then_some((version, r))
        })
        .max_by(|(a, _), (b, _)| a.cmp_precedence(b))
}

fn resolve_manifest_asset(manifest_bytes: &[u8], platform_key: &str) -> Option<ResolvedAsset> {
    let manifest: UpdateManifest = serde_json::from_slice(manifest_bytes).ok()?;
    let entry = manifest.platforms.get(platform_key)?;
    if !is_valid_download_url(&entry.url) {
        return None;
    }
    let name = entry
        .name
        .clone()
        .or_else(|| entry.url.rsplit('/').next().map(percent_decode))
        .filter(|s| !s.is_empty())?;
    let sha256 = entry
        .sha256
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    Some(ResolvedAsset {
        name,
        download_url: entry.url.clone(),
        sha256,
        size: entry.size,
    })
}

fn asset_matches_platform(filename: &str, platform_key: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    match platform_key {
        "darwin-aarch64" => lower.ends_with("_aarch64.dmg") || lower.ends_with("_arm64.dmg"),
        "darwin-x86_64" => lower.ends_with("_x64.dmg") || lower.ends_with("_x86_64.dmg"),
        "windows-x86_64" => lower.ends_with("_x64-setup.exe"),
        "windows-aarch64" => lower.ends_with("_arm64-setup.exe"),
        _ => false,
    }
}

fn parse_expanded_assets_html(html: &str, tag: &str, platform_key: &str) -> Option<ResolvedAsset> {
    let prefix = format!("/{REPO_PATH}/releases/download/{tag}/");
    let mut cursor = html;
    while let Some(idx) = cursor.find(&prefix) {
        let rest = &cursor[idx + prefix.len()..];
        let end = rest
            .find(['"', '\'', '<', '>', ' ', '\n', '\r'])
            .unwrap_or(rest.len());
        let raw_name = &rest[..end];
        if !raw_name.is_empty() && !raw_name.contains('/') {
            let decoded_name = percent_decode(raw_name);
            if asset_matches_platform(&decoded_name, platform_key) {
                let download_url = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{raw_name}");
                if is_valid_download_url(&download_url) {
                    return Some(ResolvedAsset {
                        name: decoded_name,
                        download_url,
                        sha256: None,
                        size: None,
                    });
                }
            }
        }
        cursor = &rest[end..];
    }
    None
}

fn build_http_client(current: &Version, timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("floating-todo/{current}"))
        .timeout(timeout)
        .build()
        .map_err(|_| "无法创建更新检查连接".to_string())
}

async fn fetch_releases_via_latest_redirect(
    client: &reqwest::Client,
) -> Result<Vec<Release>, String> {
    let response = client
        .get(RELEASES_LATEST_URL)
        .send()
        .await
        .map_err(|_| "无法连接 GitHub，请检查网络后重试".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            403 | 429 => "GitHub 请求受限，请稍后重试".to_string(),
            404 => "无法访问更新仓库，请确认仓库已公开".to_string(),
            _ => format!("更新检查失败（HTTP {status}）"),
        });
    }
    let final_url = response.url().as_str();
    let Some(tag_name) = extract_tag_from_release_url(final_url) else {
        return Ok(Vec::new());
    };
    let prerelease = Version::parse(tag_name.strip_prefix('v').unwrap_or(&tag_name))
        .map(|v| !v.pre.is_empty())
        .unwrap_or(false);
    Ok(vec![Release {
        tag_name,
        draft: false,
        prerelease,
    }])
}

async fn resolve_asset_for_tag(
    client: &reqwest::Client,
    tag: &str,
    platform_key: &str,
) -> Option<ResolvedAsset> {
    let manifest_url = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{MANIFEST_FILENAME}");
    if let Ok(resp) = client.get(&manifest_url).send().await {
        if resp.status().is_success() {
            if let Ok(bytes) = resp.bytes().await {
                if let Some(asset) = resolve_manifest_asset(&bytes, platform_key) {
                    return Some(asset);
                }
            }
        }
    }

    let expanded_url = format!("{RELEASES_URL}/expanded_assets/{tag}");
    if let Ok(resp) = client
        .get(&expanded_url)
        .header("Accept", "text/html")
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(html) = resp.text().await {
                if let Some(asset) = parse_expanded_assets_html(&html, tag, platform_key) {
                    return Some(asset);
                }
            }
        }
    }

    None
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

fn emit_progress(
    app: &tauri::AppHandle,
    stage: &str,
    downloaded: u64,
    total: Option<u64>,
    message: impl Into<String>,
) {
    let percent = total
        .filter(|&t| t > 0)
        .map(|t| ((downloaded.saturating_mul(100)) / t).min(100) as u8);
    let _ = app.emit(
        "update-progress",
        UpdateProgress {
            stage: stage.to_string(),
            downloaded,
            total,
            percent,
            message: message.into(),
        },
    );
}

#[cfg(any(target_os = "macos", test))]
fn find_app_bundle_path(exe_path: &Path) -> Option<PathBuf> {
    exe_path
        .ancestors()
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("app"))
        .map(Path::to_path_buf)
}

#[cfg(target_os = "macos")]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
struct MountGuard {
    mount_dir: PathBuf,
    mounted: bool,
}

#[cfg(target_os = "macos")]
impl Drop for MountGuard {
    fn drop(&mut self) {
        if self.mounted {
            let status = std::process::Command::new("hdiutil")
                .args(["detach", "-quiet"])
                .arg(&self.mount_dir)
                .status();
            if !matches!(status, Ok(s) if s.success()) {
                let _ = std::process::Command::new("hdiutil")
                    .args(["detach", "-force", "-quiet"])
                    .arg(&self.mount_dir)
                    .status();
            }
            self.mounted = false;
        }
        let _ = std::fs::remove_dir(&self.mount_dir);
    }
}

#[cfg(target_os = "macos")]
fn install_macos_dmg(
    app: &tauri::AppHandle,
    dmg_path: &Path,
    updates_dir: &Path,
) -> Result<(), String> {
    let current_exe =
        std::env::current_exe().map_err(|_| "无法获取当前应用路径".to_string())?;
    let current_app = find_app_bundle_path(&current_exe)
        .ok_or_else(|| "当前处于开发模式，无法原地替换应用包".to_string())?;
    let parent_dir = current_app
        .parent()
        .ok_or_else(|| "无法定位应用安装目录".to_string())?;

    let pid = std::process::id();
    let mount_dir = updates_dir.join(format!("mount-{pid}"));
    let _ = std::fs::remove_dir_all(&mount_dir);
    std::fs::create_dir_all(&mount_dir).map_err(|_| "无法创建临时挂载目录".to_string())?;

    let attach = std::process::Command::new("hdiutil")
        .arg("attach")
        .arg(dmg_path)
        .args(["-readonly", "-nobrowse", "-quiet", "-mountpoint"])
        .arg(&mount_dir)
        .output()
        .map_err(|_| "无法调用 hdiutil 挂载安装包".to_string())?;
    if !attach.status.success() {
        let _ = std::fs::remove_dir_all(&mount_dir);
        return Err("挂载更新镜像失败，安装包可能已损坏".to_string());
    }
    let guard = MountGuard {
        mount_dir: mount_dir.clone(),
        mounted: true,
    };

    let entries = std::fs::read_dir(&mount_dir)
        .map_err(|_| "无法读取挂载的安装包内容".to_string())?;
    let mut mounted_app = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.extension().and_then(|e| e.to_str()) == Some("app") {
            mounted_app = Some(path);
            break;
        }
    }
    let mounted_app =
        mounted_app.ok_or_else(|| "安装包中未找到应用程序（.app）".to_string())?;

    let info_plist = mounted_app.join("Contents").join("Info.plist");
    let plist_out = std::process::Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleIdentifier"])
        .arg(&info_plist)
        .output()
        .map_err(|_| "无法校验安装包签名信息".to_string())?;
    let bundle_id = String::from_utf8_lossy(&plist_out.stdout).trim().to_string();
    if !plist_out.status.success() || bundle_id != EXPECTED_BUNDLE_ID {
        return Err(format!("安装包标识不匹配（{bundle_id}），已中止更新"));
    }

    let direct_stage = parent_dir.join(format!(".floating-todo-update-{pid}.app"));
    let backup_app = parent_dir.join(format!(".floating-todo-backup-{pid}.app"));
    let can_write_parent = std::fs::create_dir_all(&direct_stage).is_ok();
    if can_write_parent {
        let _ = std::fs::remove_dir_all(&direct_stage);
    }

    let staged_app = if can_write_parent {
        direct_stage.clone()
    } else {
        let alt = updates_dir.join(format!("staged-{pid}.app"));
        let _ = std::fs::remove_dir_all(&alt);
        alt
    };

    let ditto = std::process::Command::new("ditto")
        .arg(&mounted_app)
        .arg(&staged_app)
        .output()
        .map_err(|_| "复制新版本应用失败".to_string())?;
    if !ditto.status.success() {
        let _ = std::fs::remove_dir_all(&staged_app);
        return Err("复制新版本应用失败，请检查磁盘空间".to_string());
    }

    let _ = std::process::Command::new("xattr")
        .args(["-rd", "com.apple.quarantine"])
        .arg(&staged_app)
        .output();

    // Unmount the DMG before swapping the application bundle.
    drop(guard);

    let cleanup_backup = if can_write_parent {
        let _ = std::fs::remove_dir_all(&backup_app);
        std::fs::rename(&current_app, &backup_app).map_err(|e| {
            let _ = std::fs::remove_dir_all(&staged_app);
            format!("无法备份当前应用：{e}")
        })?;
        if let Err(e) = std::fs::rename(&staged_app, &current_app) {
            let _ = std::fs::rename(&backup_app, &current_app);
            let _ = std::fs::remove_dir_all(&staged_app);
            return Err(format!("替换应用失败：{e}"));
        }
        backup_app
    } else {
        let cmd = format!(
            "rm -rf {bak} && mv {cur} {bak} && (mv {new} {cur} || (mv {bak} {cur}; exit 1)) && rm -rf {bak}",
            bak = shell_quote(&backup_app.to_string_lossy()),
            cur = shell_quote(&current_app.to_string_lossy()),
            new = shell_quote(&staged_app.to_string_lossy()),
        );
        let osa_script = format!(
            "do shell script \"{}\" with administrator privileges",
            cmd.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let status = std::process::Command::new("osascript")
            .args(["-e", &osa_script])
            .output()
            .map_err(|_| "无法请求管理员权限替换应用".to_string())?;
        if !status.status.success() {
            let _ = std::fs::remove_dir_all(&staged_app);
            return Err("已取消或未能获取管理员授权，未替换应用".to_string());
        }
        PathBuf::new()
    };

    emit_progress(app, "restarting", 100, Some(100), "更新完成，正在重启应用…");

    let relaunch_script = r#"pid="$1"; app="$2"; backup="$3"; dmg="$4"
i=0
while kill -0 "$pid" 2>/dev/null && [ "$i" -lt 100 ]; do
  sleep 0.1
  i=$((i + 1))
done
if [ -n "$backup" ]; then rm -rf "$backup" 2>/dev/null || true; fi
if [ -n "$dmg" ]; then rm -f "$dmg" 2>/dev/null || true; fi
open -n "$app"
"#;

    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(relaunch_script)
        .arg("--")
        .arg(pid.to_string())
        .arg(&current_app)
        .arg(&cleanup_backup)
        .arg(dmg_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "应用已替换，但启动新版本失败，请手动重新打开应用".to_string())?;

    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        handle.exit(0);
    });

    Ok(())
}

#[cfg(target_os = "windows")]
fn install_windows_nsis(app: &tauri::AppHandle, installer_path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let current_exe =
        std::env::current_exe().map_err(|_| "无法获取当前应用路径".to_string())?;
    let pid = std::process::id();

    let ps_quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let script = format!(
        "try {{ Wait-Process -Id {pid} -Timeout 30 -ErrorAction SilentlyContinue }} catch {{}}; \
         $p = Start-Process -FilePath {installer} -ArgumentList '/S' -Wait -PassThru; \
         Remove-Item -LiteralPath {installer} -Force -ErrorAction SilentlyContinue; \
         if ($p.ExitCode -eq 0 -and (Test-Path -LiteralPath {exe})) {{ Start-Process -FilePath {exe} }}",
        pid = pid,
        installer = ps_quote(&installer_path.to_string_lossy()),
        exe = ps_quote(&current_exe.to_string_lossy()),
    );

    emit_progress(app, "restarting", 100, Some(100), "正在启动安装程序并重启…");

    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "无法启动更新安装程序，请前往 Releases 手动下载".to_string())?;

    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        handle.exit(0);
    });

    Ok(())
}

#[tauri::command]
pub fn installed_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub async fn check_for_updates(
    app: tauri::AppHandle,
    state: tauri::State<'_, UpdateState>,
) -> Result<UpdateStatus, String> {
    let current = app.package_info().version.clone();
    let client = build_http_client(&current, Duration::from_secs(20))?;

    let cached_etag = { state.cache.lock().await.etag.clone() };

    let mut req = client
        .get(RELEASES_ATOM_URL)
        .header("Accept", "application/atom+xml, application/xml;q=0.9, */*;q=0.8");
    if let Some(etag) = &cached_etag {
        req = req.header("If-None-Match", etag);
    }

    let atom_result = req.send().await;
    let (releases, new_etag) = match atom_result {
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_MODIFIED => {
            if let Some(cached) = state.cache.lock().await.last_status.clone() {
                return Ok(cached);
            }
            // Cache was unexpectedly empty; fetch fresh without ETag.
            let fresh = client
                .get(RELEASES_ATOM_URL)
                .header("Accept", "application/atom+xml, application/xml;q=0.9")
                .send()
                .await
                .map_err(|_| "无法连接 GitHub，请检查网络后重试".to_string())?;
            if !fresh.status().is_success() {
                return Err(format!("更新检查失败（HTTP {}）", fresh.status()));
            }
            let etag = fresh
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let xml = fresh
                .text()
                .await
                .map_err(|_| "读取更新信息失败".to_string())?;
            (parse_atom_releases(&xml)?, etag)
        }
        Ok(resp) if resp.status().is_success() => {
            let etag = resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let xml = resp
                .text()
                .await
                .map_err(|_| "读取更新信息失败".to_string())?;
            (parse_atom_releases(&xml)?, etag)
        }
        Ok(resp) => {
            let status = resp.status();
            if current.pre.is_empty() {
                (fetch_releases_via_latest_redirect(&client).await?, None)
            } else {
                return Err(match status.as_u16() {
                    403 | 429 => "GitHub 请求受限，请稍后重试".to_string(),
                    404 => "无法访问更新仓库，请确认仓库已公开".to_string(),
                    _ => format!("更新检查失败（HTTP {status}）"),
                });
            }
        }
        Err(_) => {
            if current.pre.is_empty() {
                (fetch_releases_via_latest_redirect(&client).await?, None)
            } else {
                return Err("无法连接 GitHub，请检查网络后重试".to_string());
            }
        }
    };

    let selected = select_release(&current, &releases);
    let (version, url, asset) = if let Some((ver, rel)) = selected {
        let tag = &rel.tag_name;
        let url = format!("{RELEASES_URL}/tag/{tag}");
        let asset = match current_platform_key() {
            Some(key) => resolve_asset_for_tag(&client, tag, key).await,
            None => None,
        };
        (Some(ver.to_string()), url, asset)
    } else {
        (None, RELEASES_URL.to_string(), None)
    };

    let status = UpdateStatus {
        current_version: current.to_string(),
        version,
        url,
        can_install: asset.is_some(),
        asset_name: asset.as_ref().map(|a| a.name.clone()),
        asset_size: asset.as_ref().and_then(|a| a.size),
        download_url: asset.as_ref().map(|a| a.download_url.clone()),
        sha256: asset.and_then(|a| a.sha256),
    };

    {
        let mut cache = state.cache.lock().await;
        if new_etag.is_some() {
            cache.etag = new_etag;
        }
        cache.last_status = Some(status.clone());
    }

    Ok(status)
}

#[tauri::command]
pub async fn install_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, UpdateState>,
) -> Result<(), String> {
    if state
        .installing
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("正在更新中，请稍候".to_string());
    }
    let _guard = InstallGuard(&state.installing);

    let cached = { state.cache.lock().await.last_status.clone() };
    let status = match cached {
        Some(s) if s.version.is_some() && s.download_url.is_some() => s,
        _ => {
            // Re-check if cache wasn't populated yet.
            let fresh = check_for_updates(app.clone(), state.clone()).await?;
            if fresh.version.is_none() {
                return Err("当前已是最新版本".to_string());
            }
            fresh
        }
    };

    let target_version = status
        .version
        .as_deref()
        .ok_or_else(|| "当前已是最新版本".to_string())?;
    let download_url = status
        .download_url
        .as_deref()
        .filter(|u| is_valid_download_url(u))
        .ok_or_else(|| "当前平台暂不支持应用内自动更新，请前往 Releases 手动下载".to_string())?;
    let asset_name = status
        .asset_name
        .as_deref()
        .filter(|n| !n.is_empty() && !n.contains('/') && !n.contains('\\'))
        .ok_or_else(|| "更新包文件名无效".to_string())?;

    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|_| "无法访问应用缓存目录".to_string())?;
    let updates_dir = cache_dir.join("updates");
    std::fs::create_dir_all(&updates_dir).map_err(|_| "无法创建更新缓存目录".to_string())?;

    let part_path = updates_dir.join(format!("{asset_name}.part"));
    let final_path = updates_dir.join(asset_name);
    let _ = std::fs::remove_file(&part_path);

    let current = app.package_info().version.clone();
    let client = build_http_client(&current, Duration::from_secs(300))?;

    emit_progress(
        &app,
        "downloading",
        0,
        status.asset_size,
        format!("正在连接并下载 {target_version}…"),
    );

    let mut response = client
        .get(download_url)
        .send()
        .await
        .map_err(|_| "下载更新失败，请检查网络后重试".to_string())?;
    if !response.status().is_success() {
        return Err(format!("下载更新失败（HTTP {}）", response.status()));
    }

    let total = response.content_length().or(status.asset_size);
    let mut file =
        std::fs::File::create(&part_path).map_err(|_| "无法写入临时更新文件".to_string())?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    use std::io::Write;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "下载中断，请检查网络后重试".to_string())?
    {
        file.write_all(&chunk)
            .map_err(|_| "写入更新文件失败，请检查磁盘空间".to_string())?;
        hasher.update(&chunk);
        downloaded = downloaded.saturating_add(chunk.len() as u64);

        if last_emit.elapsed() >= Duration::from_millis(120) {
            last_emit = Instant::now();
            emit_progress(
                &app,
                "downloading",
                downloaded,
                total,
                format!("正在下载新版本 {target_version}…"),
            );
        }
    }
    file.flush()
        .map_err(|_| "写入更新文件失败".to_string())?;
    drop(file);

    emit_progress(
        &app,
        "verifying",
        downloaded,
        total.or(Some(downloaded)),
        "正在校验安装包…",
    );

    if let Some(expected_sha) = &status.sha256 {
        let actual_sha = hex_encode(&hasher.finalize());
        if actual_sha != *expected_sha {
            let _ = std::fs::remove_file(&part_path);
            return Err("安装包校验失败（SHA-256 不匹配），请重试".to_string());
        }
    }

    let _ = std::fs::remove_file(&final_path);
    std::fs::rename(&part_path, &final_path).map_err(|_| "保存安装包失败".to_string())?;

    emit_progress(
        &app,
        "installing",
        downloaded,
        total.or(Some(downloaded)),
        "正在替换应用…",
    );

    #[cfg(target_os = "macos")]
    {
        let app_clone = app.clone();
        let dmg = final_path.clone();
        let dir = updates_dir.clone();
        tokio::task::spawn_blocking(move || install_macos_dmg(&app_clone, &dmg, &dir))
            .await
            .map_err(|_| "更新任务异常中断".to_string())?
    }

    #[cfg(target_os = "windows")]
    {
        install_windows_nsis(&app, &final_path)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = std::fs::remove_file(&final_path);
        Err("当前平台暂不支持应用内自动替换，请前往 Releases 手动下载".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, draft: bool) -> Release {
        Release {
            tag_name: tag.into(),
            prerelease,
            draft,
        }
    }

    #[test]
    fn selects_semantic_maximum_and_respects_channels() {
        let releases = vec![
            release("v0.4.0-rc.10", true, false),
            release("v0.4.0-rc.6", true, false),
            release("v0.3.9", false, false),
            release("v9.0.0", false, true),
            release("bad", false, false),
        ];
        assert_eq!(
            select_release(&Version::parse("0.4.0-rc.5").unwrap(), &releases)
                .unwrap()
                .0
                .to_string(),
            "0.4.0-rc.10"
        );
        assert_eq!(
            select_release(&Version::parse("0.3.8").unwrap(), &releases)
                .unwrap()
                .0
                .to_string(),
            "0.3.9"
        );
        assert!(select_release(&Version::parse("0.4.0").unwrap(), &releases).is_none());
    }

    #[test]
    fn stable_promotes_rc_and_build_metadata_is_not_an_update() {
        let releases = vec![
            release("0.4.0", false, false),
            release("v0.4.0+build.2", false, false),
        ];
        assert_eq!(
            select_release(&Version::parse("0.4.0-rc.5").unwrap(), &releases)
                .unwrap()
                .0
                .pre
                .as_str(),
            ""
        );
        assert!(select_release(&Version::parse("0.4.0+build.1").unwrap(), &releases).is_none());
        assert!(select_release(&Version::parse("0.4.0").unwrap(), &[]).is_none());
        assert!(select_release(
            &Version::parse("0.3.0").unwrap(),
            &[release("v0.5.0-rc.1", false, false)]
        )
        .is_none());
    }

    #[test]
    fn parses_github_atom_feed() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Release notes from floating-todo</title>
  <entry>
    <id>tag:github.com,2008:Repository/12345/v0.4.2-rc8</id>
    <link rel="alternate" type="text/html" href="https://github.com/pisceskkk/floating-todo/releases/tag/v0.4.2-rc8"/>
    <title>floating todo v0.4.2-rc8</title>
  </entry>
  <entry>
    <id>tag:github.com,2008:Repository/12345/v0.4.1</id>
    <link rel="alternate" type="text/html" href="https://github.com/pisceskkk/floating-todo/releases/tag/v0.4.1"/>
    <title>floating todo v0.4.1</title>
  </entry>
</feed>"#;
        let releases = parse_atom_releases(xml).unwrap();
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].tag_name, "v0.4.2-rc8");
        assert!(releases[0].prerelease);
        assert_eq!(releases[1].tag_name, "v0.4.1");
        assert!(!releases[1].prerelease);

        let stable_pick = select_release(&Version::parse("0.4.0").unwrap(), &releases).unwrap();
        assert_eq!(stable_pick.0.to_string(), "0.4.1");

        let rc_pick = select_release(&Version::parse("0.4.2-rc7").unwrap(), &releases).unwrap();
        assert_eq!(rc_pick.0.to_string(), "0.4.2-rc8");
    }

    #[test]
    fn resolves_manifest_and_validates_url_and_sha() {
        let sha = "a".repeat(64);
        let manifest = serde_json::json!({
            "version": "0.4.2",
            "platforms": {
                "darwin-aarch64": {
                    "url": "https://github.com/pisceskkk/floating-todo/releases/download/v0.4.2/floating-todo_0.4.2_aarch64.dmg",
                    "sha256": sha,
                    "size": 12345678
                },
                "windows-x86_64": {
                    "url": "https://evil.example.com/floating-todo_0.4.2_x64-setup.exe",
                    "sha256": sha
                }
            }
        });
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let mac = resolve_manifest_asset(&bytes, "darwin-aarch64").unwrap();
        assert_eq!(mac.name, "floating-todo_0.4.2_aarch64.dmg");
        assert_eq!(mac.sha256.as_deref(), Some(sha.as_str()));
        assert_eq!(mac.size, Some(12345678));

        // Rejects non-repo URLs.
        assert!(resolve_manifest_asset(&bytes, "windows-x86_64").is_none());
    }

    #[test]
    fn parses_expanded_assets_html_fallback() {
        let html = r#"
        <ul>
          <li><a href="/pisceskkk/floating-todo/releases/download/v0.4.2-rc7/floating-todo_0.4.2-rc7_aarch64.dmg">mac arm</a></li>
          <li><a href="/pisceskkk/floating-todo/releases/download/v0.4.2-rc7/floating-todo_0.4.2-rc7_x64.dmg">mac x64</a></li>
          <li><a href="/pisceskkk/floating-todo/releases/download/v0.4.2-rc7/floating-todo_0.4.2-rc7_x64-setup.exe">win</a></li>
        </ul>"#;
        let arm = parse_expanded_assets_html(html, "v0.4.2-rc7", "darwin-aarch64").unwrap();
        assert_eq!(arm.name, "floating-todo_0.4.2-rc7_aarch64.dmg");
        assert_eq!(
            arm.download_url,
            "https://github.com/pisceskkk/floating-todo/releases/download/v0.4.2-rc7/floating-todo_0.4.2-rc7_aarch64.dmg"
        );

        let win = parse_expanded_assets_html(html, "v0.4.2-rc7", "windows-x86_64").unwrap();
        assert_eq!(win.name, "floating-todo_0.4.2-rc7_x64-setup.exe");
    }

    #[test]
    fn finds_macos_app_bundle_from_executable_path() {
        let exe = Path::new("/Applications/floating todo.app/Contents/MacOS/floating-todo");
        assert_eq!(
            find_app_bundle_path(exe),
            Some(PathBuf::from("/Applications/floating todo.app"))
        );
        let dev_exe = Path::new("/Users/sk/project/src-tauri/target/debug/floating-todo");
        assert_eq!(find_app_bundle_path(dev_exe), None);
    }
}
