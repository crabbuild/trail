#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessIdentityMatch {
    Match,
    DeadOrMismatch,
    Unknown,
}

#[cfg(test)]
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
    #[cfg(target_os = "macos")]
    {
        return macos_process_start_token(pid);
    }
    #[cfg(target_os = "freebsd")]
    {
        return freebsd_process_start_token(pid);
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

#[cfg(test)]
pub(crate) fn different_process_start_token_for_test(pid: u32) -> String {
    let token = process_start_token(pid).expect("test process has a stable start token");
    let (prefix, last_component) = token
        .rsplit_once(':')
        .expect("canonical process start token has a numeric component");
    let value = last_component
        .parse::<u64>()
        .expect("canonical process start token ends in a number");
    let different = if token.starts_with("macos:") || token.starts_with("freebsd:") {
        (value + 1) % 1_000_000
    } else {
        value
            .checked_add(1)
            .unwrap_or_else(|| value.saturating_sub(1))
    };
    format!("{prefix}:{different}")
}

#[cfg(target_os = "macos")]
fn macos_process_start_token(pid: u32) -> Option<String> {
    let raw_pid = i32::try_from(pid).ok()?;
    let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
    let expected = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: `info` points to a writable `proc_bsdinfo` allocation and the
    // supplied byte count exactly matches that allocation.
    let read = unsafe {
        libc::proc_pidinfo(
            raw_pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            expected,
        )
    };
    if read != expected || info.pbi_pid != pid || info.pbi_start_tvusec >= 1_000_000 {
        return None;
    }
    Some(format!(
        "macos:{}:{}:{}",
        info.pbi_pid, info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}

#[cfg(target_os = "freebsd")]
fn freebsd_process_start_token(pid: u32) -> Option<String> {
    let raw_pid = i32::try_from(pid).ok()?;
    let mut info = unsafe { std::mem::zeroed::<libc::kinfo_proc>() };
    let mut info_len = std::mem::size_of::<libc::kinfo_proc>();
    let mib = [
        libc::CTL_KERN,
        libc::KERN_PROC,
        libc::KERN_PROC_PID,
        raw_pid,
    ];
    // SAFETY: the MIB contains the documented KERN_PROC_PID query, `info`
    // points to a writable `kinfo_proc`, and `info_len` describes its size.
    let result = unsafe {
        libc::sysctl(
            mib.as_ptr(),
            mib.len() as libc::c_uint,
            (&mut info as *mut libc::kinfo_proc).cast(),
            &mut info_len,
            std::ptr::null(),
            0,
        )
    };
    if result != 0
        || info_len != std::mem::size_of::<libc::kinfo_proc>()
        || info.ki_pid != raw_pid
        || info.ki_start.tv_sec < 0
        || !(0..1_000_000).contains(&info.ki_start.tv_usec)
    {
        return None;
    }
    Some(format!(
        "freebsd:{}:{}:{}",
        info.ki_pid, info.ki_start.tv_sec, info.ki_start.tv_usec
    ))
}

#[cfg(all(test, target_os = "windows"))]
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
    process_identity_may_match(process_start_token_match(pid, token))
}

pub(crate) fn process_start_token_match(pid: u32, token: &str) -> ProcessIdentityMatch {
    #[cfg(unix)]
    {
        unix_process_start_token_match(pid, token)
    }

    #[cfg(target_os = "windows")]
    {
        windows_process_start_token_match(pid, token)
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = (pid, token);
        ProcessIdentityMatch::Unknown
    }
}

fn process_identity_may_match(result: ProcessIdentityMatch) -> bool {
    result != ProcessIdentityMatch::DeadOrMismatch
}

/// Internal helper entry point used by the CLI's hidden process-watchdog mode.
/// The watchdog is a direct child of the Trail process and owns no workspace
/// state. It terminates one authenticated sandbox-helper process if its parent
/// disappears, preventing an adapter action from surviving host death.
#[doc(hidden)]
pub fn run_internal_process_watchdog(
    parent_pid: u32,
    child_pid: u32,
    child_start_token: &str,
) -> std::result::Result<(), String> {
    if child_pid == 0 || child_start_token.is_empty() {
        return Err("process watchdog received an invalid child identity".to_string());
    }
    if !process_matches_start_token(child_pid, child_start_token) {
        return Ok(());
    }

    #[cfg(unix)]
    {
        let parent_pid = i32::try_from(parent_pid)
            .map_err(|_| "process watchdog parent PID is out of range".to_string())?;
        let child_pid = i32::try_from(child_pid)
            .map_err(|_| "process watchdog child PID is out of range".to_string())?;
        loop {
            if !process_matches_start_token(child_pid as u32, child_start_token) {
                return Ok(());
            }
            // The watchdog is spawned directly by the owning Trail process.
            // Unix reparents it immediately if that process exits, avoiding
            // repeated process-table probes and PID-reuse ambiguity.
            if unsafe { libc::getppid() } != parent_pid {
                // SAFETY: the PID was range-checked and still matches the
                // captured start token immediately before this signal.
                if unsafe { libc::kill(child_pid, libc::SIGKILL) } != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(format!(
                            "process watchdog could not terminate child {child_pid}: {error}"
                        ));
                    }
                }
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    #[cfg(target_os = "windows")]
    {
        return windows_watch_parent_and_terminate_child(parent_pid, child_pid, child_start_token);
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = parent_pid;
        Err("process watchdog is unavailable on this platform".to_string())
    }
}

#[cfg(target_os = "windows")]
fn windows_watch_parent_and_terminate_child(
    parent_pid: u32,
    child_pid: u32,
    child_start_token: &str,
) -> std::result::Result<(), String> {
    use winapi::shared::minwindef::FALSE;
    use winapi::shared::winerror::WAIT_TIMEOUT;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{OpenProcess, TerminateProcess};
    use winapi::um::synchapi::WaitForSingleObject;
    use winapi::um::winbase::WAIT_OBJECT_0;
    use winapi::um::winnt::{PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, SYNCHRONIZE};

    // SAFETY: handles are checked before use and closed on every return path.
    unsafe {
        let parent = OpenProcess(SYNCHRONIZE, FALSE, parent_pid);
        if parent.is_null() {
            return Err(format!(
                "process watchdog cannot open parent {parent_pid}: {}",
                std::io::Error::last_os_error()
            ));
        }
        let child = OpenProcess(
            SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            FALSE,
            child_pid,
        );
        if child.is_null() {
            CloseHandle(parent);
            return Ok(());
        }
        if windows_process_start_token(child_pid).as_deref() != Some(child_start_token) {
            CloseHandle(child);
            CloseHandle(parent);
            return Ok(());
        }
        loop {
            if WaitForSingleObject(child, 0) == WAIT_OBJECT_0 {
                CloseHandle(child);
                CloseHandle(parent);
                return Ok(());
            }
            match WaitForSingleObject(parent, 50) {
                WAIT_OBJECT_0 => {
                    let _ = TerminateProcess(child, 137);
                    CloseHandle(child);
                    CloseHandle(parent);
                    return Ok(());
                }
                WAIT_TIMEOUT => {}
                _ => {
                    let error = std::io::Error::last_os_error();
                    CloseHandle(child);
                    CloseHandle(parent);
                    return Err(format!("process watchdog wait failed: {error}"));
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_process_start_token_match(pid: u32, token: &str) -> ProcessIdentityMatch {
    use winapi::shared::minwindef::{DWORD, FILETIME};
    use winapi::shared::winerror::ERROR_INVALID_PARAMETER;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{GetExitCodeProcess, GetProcessTimes, OpenProcess};
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;

    const STILL_ACTIVE: DWORD = 259;
    // SAFETY: the handle is checked before use and closed on every path after opening.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return if GetLastError() == ERROR_INVALID_PARAMETER {
                ProcessIdentityMatch::DeadOrMismatch
            } else {
                ProcessIdentityMatch::Unknown
            };
        }
        let mut exit_code = 0;
        if GetExitCodeProcess(handle, &mut exit_code) == 0 {
            CloseHandle(handle);
            return ProcessIdentityMatch::Unknown;
        }
        if exit_code != STILL_ACTIVE {
            CloseHandle(handle);
            return ProcessIdentityMatch::DeadOrMismatch;
        }

        let mut created: FILETIME = std::mem::zeroed();
        let mut exited: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            CloseHandle(handle);
            return ProcessIdentityMatch::Unknown;
        }
        CloseHandle(handle);
        let value = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        if format!("windows:{value}") == token {
            ProcessIdentityMatch::Match
        } else if windows_process_start_token_is_comparable(token) {
            ProcessIdentityMatch::DeadOrMismatch
        } else {
            ProcessIdentityMatch::Unknown
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_process_start_token_is_comparable(token: &str) -> bool {
    token
        .strip_prefix("windows:")
        .is_some_and(canonical_decimal)
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

#[cfg(all(test, unix))]
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

#[cfg(unix)]
fn unix_process_start_token_match(pid: u32, token: &str) -> ProcessIdentityMatch {
    let Ok(raw_pid) = i32::try_from(pid) else {
        return ProcessIdentityMatch::DeadOrMismatch;
    };
    let Some(pid) = rustix::process::Pid::from_raw(raw_pid) else {
        return ProcessIdentityMatch::DeadOrMismatch;
    };

    match rustix::process::test_kill_process(pid) {
        Ok(()) | Err(rustix::io::Errno::PERM) => {}
        Err(rustix::io::Errno::SRCH) => return ProcessIdentityMatch::DeadOrMismatch,
        Err(_) => return ProcessIdentityMatch::Unknown,
    }
    match process_start_token(pid.as_raw_nonzero().get() as u32) {
        Some(actual) if actual == token => ProcessIdentityMatch::Match,
        Some(_) if process_start_token_is_comparable(token) => ProcessIdentityMatch::DeadOrMismatch,
        // A live process with a token from an older implementation or another
        // operating system cannot be compared safely. In particular, Darwin's
        // former locale- and timezone-sensitive `ps:` token must not authorize
        // takeover merely because the representation changed.
        Some(_) => ProcessIdentityMatch::Unknown,
        None => ProcessIdentityMatch::Unknown,
    }
}

#[cfg(unix)]
fn process_start_token_is_comparable(token: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        return token.strip_prefix("linux:").is_some_and(canonical_decimal);
    }
    #[cfg(target_os = "macos")]
    {
        return canonical_bsd_start_token(token, "macos");
    }
    #[cfg(target_os = "freebsd")]
    {
        return canonical_bsd_start_token(token, "freebsd");
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]
    {
        let _ = token;
        false
    }
}

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
fn canonical_bsd_start_token(token: &str, platform: &str) -> bool {
    let mut fields = token.split(':');
    let (Some(found_platform), Some(pid), Some(seconds), Some(microseconds), None) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return false;
    };
    found_platform == platform
        && canonical_decimal(pid)
        && canonical_decimal(seconds)
        && canonical_decimal(microseconds)
        && microseconds
            .parse::<u32>()
            .is_ok_and(|value| value < 1_000_000)
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "windows"
))]
fn canonical_decimal(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|parsed| parsed.to_string() == value)
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
        assert_eq!(
            process_start_token_match(std::process::id(), &token),
            ProcessIdentityMatch::Match
        );
    }

    #[test]
    fn unknown_identity_probe_is_conservatively_live_for_boolean_callers() {
        assert!(process_identity_may_match(ProcessIdentityMatch::Unknown));
        assert!(process_identity_may_match(ProcessIdentityMatch::Match));
        assert!(!process_identity_may_match(
            ProcessIdentityMatch::DeadOrMismatch
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn live_process_with_local_fallback_token_is_indeterminate() {
        let pid = std::process::id();
        assert_eq!(
            process_start_token_match(pid, &format!("local:{pid}:1")),
            ProcessIdentityMatch::Unknown
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn dead_process_with_local_fallback_token_is_reclaimable() {
        assert_eq!(
            process_start_token_match(0, "local:0:1"),
            ProcessIdentityMatch::DeadOrMismatch
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_start_token_is_stable_across_ps_timezones() {
        let pid = std::process::id();
        let first = process_start_token(pid).expect("current process has a start token");
        let ps_start = |timezone: &str| {
            let output = std::process::Command::new("ps")
                .env("TZ", timezone)
                .args(["-o", "lstart=", "-p", &pid.to_string()])
                .output()
                .expect("query ps start time");
            assert!(output.status.success());
            String::from_utf8(output.stdout)
                .expect("ps output is UTF-8")
                .trim()
                .to_string()
        };
        let utc_ps_start = ps_start("UTC");
        let vancouver_ps_start = ps_start("America/Vancouver");
        let second = process_start_token(pid).expect("current process still has a start token");

        assert_ne!(utc_ps_start, vancouver_ps_start);
        assert_eq!(first, second);
        assert!(first.starts_with(&format!("macos:{pid}:")));
        assert!(canonical_bsd_start_token(&first, "macos"));
        assert!(!first.starts_with("ps:"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_process_with_legacy_ps_token_is_indeterminate() {
        assert_eq!(
            process_start_token_match(std::process::id(), "ps:Mon Jul 20 00:00:00 2026"),
            ProcessIdentityMatch::Unknown
        );
    }
}
