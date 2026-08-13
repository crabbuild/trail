#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use trail::{InitImportMode, Trail};

fn trail_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_trail"))
}

fn initialize_workspace() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("README.md"), "colima runtime\n").unwrap();
    Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    workspace
}

fn write_executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).unwrap();
}

fn fake_path(bin: &Path) -> std::ffi::OsString {
    let mut paths = vec![bin.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).unwrap()
}

fn write_fake_limactl(bin: &Path) {
    write_executable(&bin.join("limactl"), "#!/bin/sh\nexit 0\n");
}

#[test]
fn runtime_config_defaults_and_validates_provider_and_profile() {
    let workspace = initialize_workspace();
    let mut db = Trail::open(workspace.path()).unwrap();

    assert_eq!(db.config_get("runtime.provider").unwrap().value, "auto");
    assert_eq!(db.config_get("runtime.colima_profile").unwrap().value, "");
    assert_eq!(
        db.config_get("runtime.colima_autostart").unwrap().value,
        "true"
    );

    db.config_set("runtime.provider", "colima").unwrap();
    db.config_set("runtime.colima_profile", "trail-project")
        .unwrap();
    db.config_set("runtime.colima_autostart", "false").unwrap();
    let reopened = Trail::open(workspace.path()).unwrap();
    assert_eq!(reopened.config().runtime.provider, "colima");
    assert_eq!(
        reopened.config().runtime.colima_profile.as_deref(),
        Some("trail-project")
    );
    assert!(!reopened.config().runtime.colima_autostart);

    assert!(db.config_set("runtime.provider", "lima").is_err());
    assert!(db
        .config_set("runtime.colima_profile", "../escape")
        .is_err());
}

#[test]
fn setup_starts_contained_colima_and_uses_only_its_explicit_context() {
    let workspace = initialize_workspace();
    let fake = tempfile::tempdir().unwrap();
    let colima_log = fake.path().join("colima.log");
    let docker_log = fake.path().join("docker.log");
    write_fake_limactl(fake.path());
    write_executable(
        &fake.path().join("colima"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\ncase \" $* \" in\n  *' status '*) exit 1 ;;\n  *' start '*) exit 0 ;;\nesac\nexit 2\n",
            colima_log.display()
        ),
    );
    write_executable(
        &fake.path().join("docker"),
        &format!(
            "#!/bin/sh\n[ -z \"${{DOCKER_HOST:-}}\" ] || exit 19\nprintf '%s\\n' \"$*\" >> {}\n[ \"$1\" = --context ] || exit 20\n[ \"$2\" = colima-trail-e2e ] || exit 21\n[ \"$3\" = info ] || exit 22\nprintf '\"fake-server\"\\n'\n",
            docker_log.display()
        ),
    );

    let output = Command::new(trail_bin())
        .current_dir(workspace.path())
        .args([
            "--format",
            "json",
            "env",
            "runtime",
            "setup",
            "colima",
            "--profile",
            "trail-e2e",
        ])
        .env("PATH", fake_path(fake.path()))
        .env("HOME", fake.path().join("home"))
        .env("DOCKER_HOST", "tcp://wrong.example.invalid:2375")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "setup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["provider"], "colima");
    assert_eq!(report["profile"], "trail-e2e");
    assert_eq!(report["docker_context"], "colima-trail-e2e");
    assert_eq!(report["status"], "ready");
    assert_eq!(report["started"], true);
    assert_eq!(report["containment"], "trail_no_host_mounts_v1");
    assert_eq!(report["toolchain_source"], "system");
    assert_eq!(report["toolchain_version"], serde_json::Value::Null);

    let colima_args = fs::read_to_string(colima_log).unwrap();
    assert!(colima_args.contains("--profile trail-e2e status --json"));
    assert!(colima_args.contains("--profile trail-e2e start"));
    for required in [
        "--runtime=docker",
        "--mount=none",
        "--activate=false",
        "--ssh-config=false",
        "--ssh-agent=false",
        "--kubernetes=false",
        "--network-address=false",
    ] {
        assert!(colima_args.contains(required), "missing {required}");
    }
    assert_eq!(
        fs::read_to_string(docker_log).unwrap().trim(),
        "--context colima-trail-e2e info --format {{json .ServerVersion}}"
    );

    let reopened = Trail::open(workspace.path()).unwrap();
    assert_eq!(reopened.config().runtime.provider, "colima");
    assert_eq!(
        reopened.config().runtime.colima_profile.as_deref(),
        Some("trail-e2e")
    );
    assert!(reopened.config().runtime.colima_autostart);
}

#[test]
fn failed_colima_preflight_does_not_publish_configuration() {
    let workspace = initialize_workspace();
    let fake = tempfile::tempdir().unwrap();
    write_fake_limactl(fake.path());
    write_executable(
        &fake.path().join("colima"),
        "#!/bin/sh\ncase \" $* \" in\n  *' status '*) exit 1 ;;\n  *' start '*) echo 'injected startup failure' >&2; exit 7 ;;\nesac\nexit 2\n",
    );
    write_executable(&fake.path().join("docker"), "#!/bin/sh\nexit 8\n");

    let output = Command::new(trail_bin())
        .current_dir(workspace.path())
        .args([
            "--format",
            "json",
            "env",
            "runtime",
            "setup",
            "colima",
            "--profile",
            "trail-failure",
        ])
        .env("PATH", fake_path(fake.path()))
        .env("HOME", fake.path().join("home"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("injected startup failure"));

    let reopened = Trail::open(workspace.path()).unwrap();
    assert_eq!(reopened.config().runtime.provider, "auto");
    assert!(reopened.config().runtime.colima_profile.is_none());
}

#[test]
fn no_start_setup_is_explicitly_unverified_and_status_is_read_only() {
    let workspace = initialize_workspace();
    let fake = tempfile::tempdir().unwrap();
    write_fake_limactl(fake.path());
    write_executable(&fake.path().join("colima"), "#!/bin/sh\nexit 1\n");
    write_executable(&fake.path().join("docker"), "#!/bin/sh\nexit 1\n");

    let setup = Command::new(trail_bin())
        .current_dir(workspace.path())
        .args([
            "--format",
            "json",
            "env",
            "runtime",
            "setup",
            "colima",
            "--profile",
            "trail-stopped",
            "--no-start",
        ])
        .env("PATH", fake_path(fake.path()))
        .env("HOME", fake.path().join("home"))
        .output()
        .unwrap();
    assert!(setup.status.success());
    let setup: serde_json::Value = serde_json::from_slice(&setup.stdout).unwrap();
    assert_eq!(setup["status"], "configured");
    assert_eq!(setup["autostart"], false);
    assert_eq!(setup["containment"], "not_verified");
    assert_eq!(setup["toolchain_source"], "system");

    let status = Command::new(trail_bin())
        .current_dir(workspace.path())
        .args(["--format", "json", "env", "runtime", "provider", "status"])
        .env("PATH", fake_path(fake.path()))
        .env("HOME", fake.path().join("home"))
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["status"], "stopped");
    assert_eq!(status["profile"], "trail-stopped");
    assert_eq!(status["started"], false);
}
