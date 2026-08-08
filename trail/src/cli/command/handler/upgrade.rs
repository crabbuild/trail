use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axoupdater::{AxoUpdater, Version};
use serde::{Deserialize, Serialize};

use super::*;

const APP_NAME: &str = "trail";
const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const UPDATE_LOCK_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/crabbuild/trail/releases/latest";
const HOMEBREW_FORMULA: &str = "crabbuild/tap/trail";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UpdateCache {
    checked_at_unix: u64,
    latest_version: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
}

#[derive(Debug, Serialize)]
struct UpgradeReport {
    status: &'static str,
    current_version: String,
    latest_version: String,
    installation: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallMethod {
    Homebrew,
    CargoDist,
    Unmanaged,
}

impl InstallMethod {
    fn label(self) -> &'static str {
        match self {
            Self::Homebrew => "homebrew",
            Self::CargoDist => "cargo-dist",
            Self::Unmanaged => "unmanaged",
        }
    }
}

pub(super) fn handle_upgrade_command(ctx: &RuntimeContext, args: &UpgradeArgs) -> Result<()> {
    if args.check {
        let cache = fetch_latest_release()?;
        let _ = write_update_cache(&cache);
        return render_upgrade_report(
            ctx,
            UpgradeReport {
                status: if update_available(&cache.latest_version) {
                    "update_available"
                } else {
                    "up_to_date"
                },
                current_version: env!("CARGO_PKG_VERSION").to_string(),
                latest_version: cache.latest_version,
                installation: detect_install_method().label(),
            },
        );
    }

    match detect_install_method() {
        InstallMethod::Homebrew => upgrade_with_homebrew(ctx),
        InstallMethod::CargoDist => upgrade_with_cargo_dist(ctx),
        InstallMethod::Unmanaged => Err(Error::InvalidInput(
            "this Trail executable is not managed by Homebrew or a Trail release installer; \
             reinstall with `brew install crabbuild/tap/trail` or the installer from \
             https://github.com/crabbuild/trail/releases/latest"
                .to_string(),
        )),
    }
}

pub(super) fn maybe_notify_about_update(ctx: &RuntimeContext) -> Result<()> {
    if update_checks_disabled()
        || !ctx.render.progress_allowed()
        || ctx.json
        || !matches!(ctx.format, OutputFormat::Human | OutputFormat::Plain)
        || std::env::var_os("CI").is_some()
    {
        return Ok(());
    }

    let Some(cache_path) = update_cache_path() else {
        return Ok(());
    };
    let cache = read_update_cache(&cache_path);
    if cache.as_ref().is_some_and(cache_is_fresh) {
        if let Some(cache) = cache.filter(|cache| update_available(&cache.latest_version)) {
            render_document(
                &TerminalDocument::new("Trail update available", UiTone::Attention)
                    .block(UiBlock::Metadata(vec![
                        ("Current".to_string(), env!("CARGO_PKG_VERSION").to_string()),
                        ("Latest".to_string(), cache.latest_version),
                    ]))
                    .next("trail upgrade", "install the latest stable release"),
                &ctx.render,
            )?;
        }
        return Ok(());
    }

    spawn_background_update_check();
    Ok(())
}

pub(super) fn handle_background_update_check() {
    if update_checks_disabled() {
        return;
    }
    let Some(cache_path) = update_cache_path() else {
        return;
    };
    let Some(cache_dir) = cache_path.parent() else {
        return;
    };
    if fs::create_dir_all(cache_dir).is_err() {
        return;
    }
    let Some(_lock) = UpdateCheckLock::acquire(cache_path.with_extension("lock")) else {
        return;
    };
    if read_update_cache(&cache_path)
        .as_ref()
        .is_some_and(cache_is_fresh)
    {
        return;
    }
    if let Ok(cache) = fetch_latest_release() {
        let _ = write_update_cache_at(&cache_path, &cache);
    }
}

fn fetch_latest_release() -> Result<UpdateCache> {
    let endpoint = std::env::var("TRAIL_UPDATE_API_URL")
        .unwrap_or_else(|_| GITHUB_LATEST_RELEASE_URL.to_string());
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(update_error)?;
    let mut request = client
        .get(endpoint)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(
            reqwest::header::USER_AGENT,
            format!("trail/{}", env!("CARGO_PKG_VERSION")),
        );
    if let Ok(token) = std::env::var("TRAIL_UPDATE_GITHUB_TOKEN")
        && !token.trim().is_empty()
    {
        request = request.bearer_auth(token);
    }
    let release_json = request
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(update_error)?
        .text()
        .map_err(update_error)?;
    let release = serde_json::from_str::<GithubRelease>(&release_json)?;
    let version = parse_version(&release.tag_name)?;
    Ok(UpdateCache {
        checked_at_unix: unix_timestamp(),
        latest_version: version.to_string(),
    })
}

fn upgrade_with_homebrew(ctx: &RuntimeContext) -> Result<()> {
    let mut command = ProcessCommand::new("brew");
    command.args(["upgrade", HOMEBREW_FORMULA]);
    if ctx.json {
        let output = command.output().map_err(|error| {
            Error::InvalidInput(format!(
                "could not run Homebrew; run `brew upgrade {HOMEBREW_FORMULA}` manually: {error}"
            ))
        })?;
        if !output.status.success() {
            return Err(Error::InvalidInput(format!(
                "Homebrew could not upgrade Trail: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    } else {
        let status = command.status().map_err(|error| {
            Error::InvalidInput(format!(
                "could not run Homebrew; run `brew upgrade {HOMEBREW_FORMULA}` manually: {error}"
            ))
        })?;
        if !status.success() {
            return Err(Error::InvalidInput(format!(
                "Homebrew could not upgrade Trail; run `brew upgrade {HOMEBREW_FORMULA}` manually"
            )));
        }
    }
    let latest = fetch_latest_release()
        .map(|cache| cache.latest_version)
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    let _ = write_update_cache(&UpdateCache {
        checked_at_unix: unix_timestamp(),
        latest_version: latest.clone(),
    });
    render_upgrade_report(
        ctx,
        UpgradeReport {
            status: "updated",
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            latest_version: latest,
            installation: InstallMethod::Homebrew.label(),
        },
    )
}

fn upgrade_with_cargo_dist(ctx: &RuntimeContext) -> Result<()> {
    let mut updater = AxoUpdater::new_for(APP_NAME);
    updater.load_receipt().map_err(|error| {
        Error::InvalidInput(format!(
            "Trail's release-installer receipt could not be loaded; rerun the latest Trail \
             installer before upgrading: {error}"
        ))
    })?;
    let receipt_matches = updater
        .check_receipt_is_for_this_executable()
        .map_err(update_error)?;
    if !receipt_matches {
        return Err(Error::InvalidInput(
            "the Trail release-installer receipt belongs to a different executable; invoke the \
             Trail binary installed by the official installer or reinstall it"
                .to_string(),
        ));
    }
    if ctx.json {
        updater.disable_installer_output();
    }
    let result = updater.run_sync().map_err(update_error)?;
    let (status, latest_version) = match result {
        Some(result) => ("updated", result.new_version.to_string()),
        None => ("up_to_date", env!("CARGO_PKG_VERSION").to_string()),
    };
    let _ = write_update_cache(&UpdateCache {
        checked_at_unix: unix_timestamp(),
        latest_version: latest_version.clone(),
    });
    render_upgrade_report(
        ctx,
        UpgradeReport {
            status,
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            latest_version,
            installation: InstallMethod::CargoDist.label(),
        },
    )
}

fn render_upgrade_report(ctx: &RuntimeContext, report: UpgradeReport) -> Result<()> {
    if ctx.json {
        return render_json(&report);
    }
    let (lead, tone) = match report.status {
        "updated" => ("Trail upgrade completed", UiTone::Success),
        "update_available" => ("Trail update available", UiTone::Attention),
        _ => ("Trail is up to date", UiTone::Success),
    };
    let mut document = TerminalDocument::new(lead, tone).block(UiBlock::Metadata(vec![
        ("Current".to_string(), report.current_version),
        ("Latest".to_string(), report.latest_version),
        ("Installation".to_string(), report.installation.to_string()),
    ]));
    if report.status == "update_available" {
        document = document.next("trail upgrade", "install the latest stable release");
    }
    render_document(&document, &ctx.render)
}

fn detect_install_method() -> InstallMethod {
    let current_exe = std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_default();
    if is_homebrew_path(&current_exe) {
        return InstallMethod::Homebrew;
    }
    let mut updater = AxoUpdater::new_for(APP_NAME);
    if updater.load_receipt().is_ok()
        && updater
            .check_receipt_is_for_this_executable()
            .unwrap_or(false)
    {
        return InstallMethod::CargoDist;
    }
    InstallMethod::Unmanaged
}

fn is_homebrew_path(path: &Path) -> bool {
    let components = path.components().collect::<Vec<_>>();
    components.windows(2).any(|pair| {
        component_text(pair[0]).is_some_and(|value| value == "Cellar")
            && component_text(pair[1]).is_some_and(|value| value == APP_NAME)
    })
}

fn component_text(component: Component<'_>) -> Option<&str> {
    match component {
        Component::Normal(value) => value.to_str(),
        _ => None,
    }
}

fn parse_version(value: &str) -> Result<Version> {
    let normalized = value.trim().strip_prefix('v').unwrap_or(value.trim());
    Version::parse(normalized).map_err(|error| {
        Error::InvalidInput(format!(
            "latest Trail release has invalid version `{value}`: {error}"
        ))
    })
}

fn update_available(latest: &str) -> bool {
    let Ok(latest) = Version::parse(latest) else {
        return false;
    };
    let Ok(current) = Version::parse(env!("CARGO_PKG_VERSION")) else {
        return false;
    };
    latest > current
}

fn update_checks_disabled() -> bool {
    std::env::var("TRAIL_NO_UPDATE_CHECK")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn update_cache_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Caches"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".cache"))
        });
    root.map(|root| root.join(APP_NAME).join("update.json"))
}

fn read_update_cache(path: &Path) -> Option<UpdateCache> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn cache_is_fresh(cache: &UpdateCache) -> bool {
    unix_timestamp().saturating_sub(cache.checked_at_unix) < UPDATE_INTERVAL.as_secs()
}

fn write_update_cache(cache: &UpdateCache) -> Result<()> {
    let path = update_cache_path().ok_or_else(|| {
        Error::InvalidInput("cannot resolve Trail's update cache directory".to_string())
    })?;
    write_update_cache_at(&path, cache)
}

fn write_update_cache_at(path: &Path, cache: &UpdateCache) -> Result<()> {
    let parent = path.parent().ok_or_else(|| Error::InvalidPath {
        path: path.display().to_string(),
        reason: "update cache path has no parent".to_string(),
    })?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".update-{}.tmp", std::process::id()));
    let mut bytes = serde_json::to_vec_pretty(cache)?;
    bytes.push(b'\n');
    fs::write(&temp, bytes)?;
    if let Err(error) = fs::rename(&temp, path) {
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        fs::remove_file(path)?;
        fs::rename(&temp, path)?;
    }
    Ok(())
}

fn spawn_background_update_check() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let mut command = ProcessCommand::new(executable);
    command
        .arg("__update-check")
        .env("TRAIL_NO_UPDATE_CHECK", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = command.spawn();
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn update_error(error: impl std::fmt::Display) -> Error {
    Error::InvalidInput(format!("Trail update failed: {error}"))
}

struct UpdateCheckLock {
    path: PathBuf,
}

impl UpdateCheckLock {
    fn acquire(path: PathBuf) -> Option<Self> {
        if let Ok(metadata) = fs::metadata(&path) {
            let stale = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > UPDATE_LOCK_TIMEOUT);
            if stale {
                let _ = fs::remove_file(&path);
            }
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()?;
        let _ = writeln!(file, "{}", std::process::id());
        Some(Self { path })
    }
}

impl Drop for UpdateCheckLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_homebrew_cellar_paths() {
        assert!(is_homebrew_path(Path::new(
            "/opt/homebrew/Cellar/trail/0.1.1/bin/trail"
        )));
        assert!(!is_homebrew_path(Path::new(
            "/Users/example/.cargo/bin/trail"
        )));
    }

    #[test]
    fn stale_cache_is_not_fresh() {
        let cache = UpdateCache {
            checked_at_unix: unix_timestamp().saturating_sub(UPDATE_INTERVAL.as_secs() + 1),
            latest_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        assert!(!cache_is_fresh(&cache));
    }

    #[test]
    fn newer_semver_is_available() {
        assert!(update_available("99.0.0"));
        assert!(!update_available(env!("CARGO_PKG_VERSION")));
        assert!(!update_available("not-a-version"));
    }

    #[test]
    fn release_tags_accept_the_v_prefix() {
        assert_eq!(parse_version("v1.2.3").unwrap().to_string(), "1.2.3");
    }

    #[test]
    fn update_cache_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested/update.json");
        let expected = UpdateCache {
            checked_at_unix: 42,
            latest_version: "1.2.3".to_string(),
        };
        write_update_cache_at(&path, &expected).unwrap();
        let actual = read_update_cache(&path).unwrap();
        assert_eq!(actual.checked_at_unix, expected.checked_at_unix);
        assert_eq!(actual.latest_version, expected.latest_version);
    }
}
