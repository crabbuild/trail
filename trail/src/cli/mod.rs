pub mod command;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod environment_sandbox;

pub(crate) fn run() {
    #[cfg(debug_assertions)]
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("__test-workspace-lock-holder"))
    {
        let arguments = std::env::args_os().skip(2).collect::<Vec<_>>();
        let result = if let [workspace] = arguments.as_slice() {
            trail::test_support::run_workspace_lock_holder(std::path::Path::new(workspace))
        } else {
            Err("expected workspace path".to_string())
        };
        match result {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("trail test workspace lock holder: {error}");
                std::process::exit(126);
            }
        }
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("__process-watchdog")) {
        let arguments = std::env::args().skip(2).collect::<Vec<_>>();
        let result = (|| -> std::result::Result<(), String> {
            if arguments.len() != 3 {
                return Err("expected parent PID, child PID, and child start token".to_string());
            }
            let parent_pid = arguments[0]
                .parse::<u32>()
                .map_err(|error| format!("invalid watchdog parent PID: {error}"))?;
            let child_pid = arguments[1]
                .parse::<u32>()
                .map_err(|error| format!("invalid watchdog child PID: {error}"))?;
            trail::db::run_internal_process_watchdog(parent_pid, child_pid, &arguments[2])
        })();
        match result {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("trail process watchdog: {error}");
                std::process::exit(126);
            }
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("__environment-sandbox"))
    {
        match environment_sandbox::run(std::env::args_os().skip(2)) {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("trail restricted environment sandbox: {error}");
                std::process::exit(126);
            }
        }
    }
    command::run();
}
