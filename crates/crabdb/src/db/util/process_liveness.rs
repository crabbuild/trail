pub(crate) fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unix_process_is_alive(pid)
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
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
    }
}
