pub(crate) fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unix_process_is_alive(pid)
    }

    #[cfg(target_os = "windows")]
    {
        windows_process_is_alive(pid)
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = pid;
        true
    }
}

pub(crate) fn process_start_token(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let end = stat.rfind(')')?;
        let fields = stat.get(end + 2..)?.split_whitespace().collect::<Vec<_>>();
        return fields.get(19).map(|value| format!("linux:{value}"));
    }
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    {
        let output = std::process::Command::new("ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        if output.status.success() {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !value.is_empty() {
                return Some(format!("ps:{value}"));
            }
        }
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        return windows_process_start_token(pid);
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "windows"
    )))]
    {
        let _ = pid;
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_process_is_alive(pid: u32) -> bool {
    use winapi::shared::minwindef::DWORD;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{GetExitCodeProcess, OpenProcess};
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;

    const STILL_ACTIVE: DWORD = 259;
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0;
        let alive = GetExitCodeProcess(handle, &mut exit_code) != 0 && exit_code == STILL_ACTIVE;
        CloseHandle(handle);
        alive
    }
}

#[cfg(target_os = "windows")]
fn windows_process_start_token(pid: u32) -> Option<String> {
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{GetProcessTimes, OpenProcess};
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        if !ok {
            return None;
        }
        let value = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        Some(format!("windows:{value}"))
    }
}

pub(crate) fn current_process_start_token() -> String {
    process_start_token(std::process::id()).unwrap_or_else(|| {
        format!(
            "local:{}:{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    })
}

pub(crate) fn process_matches_start_token(pid: u32, token: &str) -> bool {
    if !process_is_alive(pid) {
        return false;
    }
    process_start_token(pid).map_or(true, |actual| actual == token)
}

pub(crate) fn test_crash_point(name: &str) {
    #[cfg(test)]
    {
        use std::io::Write;
        use std::time::Duration;

        if std::env::var("TRAIL_TEST_CRASH_AT").as_deref() != Ok(name) {
            return;
        }
        let ready = std::env::var_os("TRAIL_TEST_CRASH_READY")
            .map(std::path::PathBuf::from)
            .expect("TRAIL_TEST_CRASH_READY must identify the crash handshake file");
        if let Some(parent) = ready.parent() {
            std::fs::create_dir_all(parent).expect("create crash handshake directory");
        }
        let mut file = std::fs::File::create(&ready).expect("create crash handshake file");
        file.write_all(name.as_bytes())
            .expect("write crash handshake file");
        file.sync_all().expect("sync crash handshake file");
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    #[cfg(not(test))]
    let _ = name;
}

#[cfg(unix)]
fn unix_process_is_alive(pid: u32) -> bool {
    let Ok(raw_pid) = i32::try_from(pid) else {
        return false;
    };
    let Some(pid) = rustix::process::Pid::from_raw(raw_pid) else {
        return false;
    };

    match rustix::process::test_kill_process(pid) {
        Ok(()) => true,
        Err(err) => err == rustix::io::Errno::PERM,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn invalid_or_special_pid_is_not_alive() {
        assert!(!process_is_alive(0));
        assert!(!process_is_alive(u32::MAX));
    }

    #[test]
    fn current_pid_is_alive() {
        assert!(process_is_alive(std::process::id()));
        let token = current_process_start_token();
        assert!(process_matches_start_token(std::process::id(), &token));
    }
}
