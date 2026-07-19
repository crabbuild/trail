#![allow(dead_code)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::Value;
use trail::{InitImportMode, Trail};

pub fn trail_bin() -> PathBuf {
    std::env::var_os("TRAIL_TEST_BIN")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_trail").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/trail"))
}

pub fn workspace() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("create ACP harness workspace");
    fs::write(temp.path().join("README.md"), "ACP conformance fixture\n")
        .expect("write workspace fixture");
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false)
        .expect("initialize Trail workspace");
    temp
}

/// Writes the platform launcher for a named reference-agent scenario.
///
/// Scenario implementations are deliberately data-driven Python programs. The
/// launcher itself is the native command required by the relay contract: a
/// POSIX shell script on Unix and a PowerShell script on Windows.
pub fn fixture_agent_command(workspace: &Path, scenario: &str, source: &str) -> Vec<String> {
    let program = workspace.join(format!("{scenario}-agent.py"));
    fs::write(&program, source).expect("write reference agent program");

    #[cfg(unix)]
    {
        let launcher = workspace.join(format!("{scenario}-agent.sh"));
        fs::write(
            &launcher,
            format!("#!/bin/sh\nset -eu\nexec python3 '{}'\n", program.display()),
        )
        .expect("write reference agent launcher");
        let mut permissions = fs::metadata(&launcher).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&launcher, permissions).unwrap();
        vec![launcher.to_string_lossy().into_owned()]
    }

    #[cfg(windows)]
    {
        let launcher = workspace.join(format!("{scenario}-agent.ps1"));
        fs::write(
            &launcher,
            format!(
                "$ErrorActionPreference = 'Stop'\npython '{}'\nexit $LASTEXITCODE\n",
                program.display()
            ),
        )
        .expect("write reference agent launcher");
        return vec![
            "powershell.exe".to_string(),
            "-NoProfile".to_string(),
            "-File".to_string(),
            launcher.to_string_lossy().into_owned(),
        ];
    }
}

pub fn spawn_relay(workspace: &Path, agent_command: &[String]) -> Child {
    assert!(!agent_command.is_empty(), "agent command must not be empty");
    Command::new(trail_bin())
        .arg("--workspace")
        .arg(workspace)
        .args(["acp", "relay", "--"])
        .args(agent_command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Trail ACP relay")
}

pub fn write_json(writer: &mut impl Write, value: &Value) {
    serde_json::to_writer(&mut *writer, value).expect("serialize ACP frame");
    writer.write_all(b"\n").expect("write ACP frame delimiter");
    writer.flush().expect("flush ACP frame");
}

pub fn read_json(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read ACP frame");
    assert!(
        !line.is_empty(),
        "relay closed before returning an ACP frame"
    );
    serde_json::from_str(line.trim_end()).expect("parse ACP frame")
}

pub fn relay_stdio(child: &mut Child) -> (impl Write + use<>, impl BufRead + use<>) {
    let stdin = child.stdin.take().expect("relay stdin");
    let stdout = BufReader::new(child.stdout.take().expect("relay stdout"));
    (stdin, stdout)
}
