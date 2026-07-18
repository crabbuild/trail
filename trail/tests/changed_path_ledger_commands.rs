#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::{Mutex, MutexGuard, OnceLock};

#[cfg(any(target_os = "linux", target_os = "macos"))]
static COMMAND_STATE: OnceLock<Mutex<()>> = OnceLock::new();

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn serial() -> MutexGuard<'static, ()> {
    COMMAND_STATE
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn status_and_record_use_one_continuous_fenced_candidate_flow() {
    let _guard = serial();
    trail::test_support::changed_path_command_flow().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn complete_prefix_keeps_ignored_tracked_files_and_filters_ignored_untracked_files() {
    let _guard = serial();
    trail::test_support::changed_path_tracked_ignored_candidate_flow().unwrap();
}
