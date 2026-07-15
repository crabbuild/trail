use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

const PERFORMANCE_METRICS_ENV: &str = "TRAIL_PERFORMANCE_METRICS";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OperationMetricsOutcome {
    Success,
    Error,
    #[default]
    CancelledOrUnclassified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationMetricsKind {
    Status,
    StatusReadOnly,
    Diff,
    Record,
}

impl OperationMetricsKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::StatusReadOnly => "status_read_only",
            Self::Diff => "diff",
            Self::Record => "record",
        }
    }
}

macro_rules! define_operation_metric_counters {
    ($($field:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub(crate) struct OperationMetricsDelta {
            $(pub(crate) $field: u64,)+
        }

        #[derive(Debug, Default)]
        struct AtomicOperationMetrics {
            $($field: AtomicU64,)+
        }

        impl AtomicOperationMetrics {
            fn add(&self, delta: OperationMetricsDelta) {
                $(saturating_atomic_add(&self.$field, delta.$field);)+
            }

            fn snapshot(&self) -> OperationMetricsDelta {
                OperationMetricsDelta {
                    $($field: self.$field.load(Ordering::Relaxed),)+
                }
            }
        }

        impl OperationMetricsDelta {
            fn saturating_sub(self, earlier: Self) -> Self {
                Self {
                    $($field: self.$field.saturating_sub(earlier.$field),)+
                }
            }
        }
    };
}

define_operation_metric_counters!(
    input_path_count,
    canonical_path_count,
    expanded_path_count,
    final_path_count,
    full_filesystem_walk_count,
    bounded_filesystem_walk_count,
    filesystem_entry_count,
    filesystem_stat_count,
    filesystem_read_count,
    filesystem_read_bytes,
    filesystem_hash_count,
    filesystem_hash_bytes,
    full_root_range_count,
    bounded_root_range_count,
    root_range_row_count,
    root_point_key_count,
    prolly_read_call_count,
    prolly_read_key_count,
    prolly_read_value_count,
    prolly_read_value_bytes,
    prolly_write_call_count,
    prolly_write_key_count,
    prolly_write_value_bytes,
    prolly_tree_batch_call_count,
    prolly_tree_batch_mutation_count,
    selected_worktree_index_sqlite_envelope_count,
    selected_worktree_index_sqlite_full_scan_count,
    selected_worktree_index_sqlite_row_read_count,
    selected_worktree_index_sqlite_row_delete_count,
    selected_worktree_index_sqlite_row_upsert_count,
    selected_worktree_index_sqlite_statement_count,
    selected_worktree_index_sqlite_transaction_count,
    selection_comparison_count,
    policy_build_count,
    policy_dependency_bytes,
    policy_dependency_file_count,
    git_subprocess_count,
    git_global_work_count,
    git_output_bytes,
    git_output_record_count,
    daemon_snapshot_bytes,
    daemon_snapshot_path_count,
    daemon_cumulative_rewrite_count,
    daemon_cumulative_rewrite_bytes,
    authoritative_candidate_count,
    ledger_row_touch_count,
    observer_tail_record_fold_count,
    reconciliation_run_count,
    manifest_bytes,
    manifest_key_comparison_count,
    journal_bytes,
    upper_work_count,
);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OperationMetricsReport {
    pub(crate) generation: u64,
    pub(crate) operation: String,
    pub(crate) outcome: OperationMetricsOutcome,
    pub(crate) input_path_count: u64,
    pub(crate) canonical_path_count: u64,
    pub(crate) expanded_path_count: u64,
    pub(crate) final_path_count: u64,
    pub(crate) full_filesystem_walk_count: u64,
    pub(crate) bounded_filesystem_walk_count: u64,
    pub(crate) filesystem_entry_count: u64,
    pub(crate) filesystem_stat_count: u64,
    pub(crate) filesystem_read_count: u64,
    pub(crate) filesystem_read_bytes: u64,
    pub(crate) filesystem_hash_count: u64,
    pub(crate) filesystem_hash_bytes: u64,
    pub(crate) full_root_range_count: u64,
    pub(crate) bounded_root_range_count: u64,
    pub(crate) root_range_row_count: u64,
    pub(crate) root_point_key_count: u64,
    /// Store read calls and requested keys are attempted work, including
    /// backend errors. Found values and bytes count only successful results.
    pub(crate) prolly_read_call_count: u64,
    pub(crate) prolly_read_key_count: u64,
    pub(crate) prolly_read_value_count: u64,
    pub(crate) prolly_read_value_bytes: u64,
    /// Store write calls, keys, and value bytes are attempted work, including
    /// backend errors. Deletes contribute a key and zero value bytes.
    pub(crate) prolly_write_call_count: u64,
    pub(crate) prolly_write_key_count: u64,
    pub(crate) prolly_write_value_bytes: u64,
    pub(crate) prolly_tree_batch_call_count: u64,
    pub(crate) prolly_tree_batch_mutation_count: u64,
    /// This completeness claim covers only the selected worktree-index sync
    /// envelope. It does not claim that every SQLite statement issued by the
    /// containing status/diff/record operation is instrumented.
    pub(crate) selected_worktree_index_sqlite_accounting_complete: bool,
    pub(crate) selected_worktree_index_sqlite_envelope_count: u64,
    /// Executions for which SQLite reported at least one FULLSCAN_STEP.
    pub(crate) selected_worktree_index_sqlite_full_scan_count: u64,
    /// Worktree-index rows decoded by exact/descendant candidate queries.
    /// The schema_meta baseline row is deliberately excluded.
    pub(crate) selected_worktree_index_sqlite_row_read_count: u64,
    /// Worktree-index row mutations made durable by a successful COMMIT.
    pub(crate) selected_worktree_index_sqlite_row_delete_count: u64,
    pub(crate) selected_worktree_index_sqlite_row_upsert_count: u64,
    /// Attempted SQL executions, including transaction control and failed
    /// mutation/COMMIT/ROLLBACK attempts.
    pub(crate) selected_worktree_index_sqlite_statement_count: u64,
    /// Selected-sync transactions whose BEGIN IMMEDIATE succeeded.
    pub(crate) selected_worktree_index_sqlite_transaction_count: u64,
    pub(crate) selection_comparison_count: u64,
    pub(crate) policy_build_count: u64,
    pub(crate) policy_dependency_bytes: u64,
    pub(crate) policy_dependency_file_count: u64,
    pub(crate) git_subprocess_count: u64,
    pub(crate) git_global_work_count: u64,
    pub(crate) git_output_bytes: u64,
    pub(crate) git_output_record_count: u64,
    /// Bytes physically read from the durable daemon snapshot. In-memory
    /// snapshots copy typed state and therefore contribute paths but zero bytes.
    pub(crate) daemon_snapshot_bytes: u64,
    pub(crate) daemon_snapshot_path_count: u64,
    /// Full serialized daemon snapshot rewrite work. These counters are
    /// cumulative outside request scopes as well as reported as scope deltas.
    pub(crate) daemon_cumulative_rewrite_count: u64,
    pub(crate) daemon_cumulative_rewrite_bytes: u64,
    pub(crate) daemon_cumulative_rewrite_count_total: u64,
    pub(crate) daemon_cumulative_rewrite_bytes_total: u64,
    pub(crate) authoritative_candidate_count: u64,
    pub(crate) ledger_row_touch_count: u64,
    pub(crate) observer_tail_record_fold_count: u64,
    pub(crate) reconciliation_run_count: u64,
    pub(crate) manifest_bytes: u64,
    pub(crate) manifest_key_comparison_count: u64,
    pub(crate) journal_bytes: u64,
    pub(crate) upper_work_count: u64,
    pub(crate) wall_time_ns: u64,
    pub(crate) rss_start_bytes: u64,
    pub(crate) rss_end_bytes: u64,
    pub(crate) rss_lifetime_high_water_bytes: u64,
}

#[derive(Debug, Default)]
pub(crate) struct OperationMetricsState {
    counters: AtomicOperationMetrics,
    daemon_rewrites: Mutex<DaemonRewriteTotals>,
    scope: Mutex<OperationScopeTracker>,
}

#[derive(Clone, Copy, Debug, Default)]
struct DaemonRewriteTotals {
    count: u64,
    bytes: u64,
}

pub(crate) struct OperationMetricsAccumulator {
    metrics: Option<Arc<OperationMetricsState>>,
    pub(crate) delta: OperationMetricsDelta,
}

#[derive(Debug, Default)]
struct OperationScopeTracker {
    generation: u64,
    depth: u64,
    active: Option<ActiveOperationScope>,
    last_report: OperationMetricsReport,
}

#[derive(Debug)]
struct ActiveOperationScope {
    generation: u64,
    operation: OperationMetricsKind,
    started: Instant,
    rss_start_bytes: u64,
    counters_start: OperationMetricsDelta,
}

pub(crate) struct OperationMetricsScope {
    state: Arc<OperationMetricsState>,
    generation: u64,
    finished: bool,
}

impl OperationMetricsState {
    pub(crate) fn add(&self, mut delta: OperationMetricsDelta) {
        if delta.daemon_cumulative_rewrite_count != 0 || delta.daemon_cumulative_rewrite_bytes != 0
        {
            let mut daemon = lock_unpoisoned(&self.daemon_rewrites);
            daemon.count = daemon
                .count
                .saturating_add(delta.daemon_cumulative_rewrite_count);
            daemon.bytes = daemon
                .bytes
                .saturating_add(delta.daemon_cumulative_rewrite_bytes);
            delta.daemon_cumulative_rewrite_count = 0;
            delta.daemon_cumulative_rewrite_bytes = 0;
        }
        self.counters.add(delta);
    }

    pub(crate) fn note_prolly_read_call(&self, key_count: usize) {
        saturating_atomic_add(&self.counters.prolly_read_call_count, 1);
        saturating_atomic_add(
            &self.counters.prolly_read_key_count,
            saturating_u64_from_usize(key_count),
        );
    }

    pub(crate) fn note_prolly_read_values<'a, I>(&self, values: I)
    where
        I: IntoIterator<Item = &'a Vec<u8>>,
    {
        let mut count = 0u64;
        let mut bytes = 0u64;
        for value in values {
            count = count.saturating_add(1);
            bytes = bytes.saturating_add(saturating_u64_from_usize(value.len()));
        }
        saturating_atomic_add(&self.counters.prolly_read_value_count, count);
        saturating_atomic_add(&self.counters.prolly_read_value_bytes, bytes);
    }

    pub(crate) fn note_prolly_write_call(&self, key_count: usize, value_bytes: usize) {
        saturating_atomic_add(&self.counters.prolly_write_call_count, 1);
        saturating_atomic_add(
            &self.counters.prolly_write_key_count,
            saturating_u64_from_usize(key_count),
        );
        saturating_atomic_add(
            &self.counters.prolly_write_value_bytes,
            saturating_u64_from_usize(value_bytes),
        );
    }

    #[cfg(test)]
    pub(crate) fn note_daemon_cumulative_rewrite(&self, bytes: usize) {
        let mut daemon = lock_unpoisoned(&self.daemon_rewrites);
        daemon.count = daemon.count.saturating_add(1);
        daemon.bytes = daemon
            .bytes
            .saturating_add(saturating_u64_from_usize(bytes));
    }

    pub(crate) fn profile<T, E>(
        self: &Arc<Self>,
        kind: OperationMetricsKind,
        operation: impl FnOnce() -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let scope = self.begin(kind);
        let result = operation();
        let outcome = if result.is_ok() {
            OperationMetricsOutcome::Success
        } else {
            OperationMetricsOutcome::Error
        };
        scope.finish(outcome);
        result
    }

    fn begin(self: &Arc<Self>, kind: OperationMetricsKind) -> OperationMetricsScope {
        let mut tracker = lock_unpoisoned(&self.scope);
        if tracker.depth == 0 {
            tracker.generation = tracker.generation.saturating_add(1);
            let (rss_start_bytes, _) = process_rss_snapshot();
            tracker.active = Some(ActiveOperationScope {
                generation: tracker.generation,
                operation: kind,
                started: Instant::now(),
                rss_start_bytes,
                counters_start: self.snapshot(),
            });
        }
        tracker.depth = tracker.depth.saturating_add(1);
        let generation = tracker
            .active
            .as_ref()
            .map(|active| active.generation)
            .unwrap_or(tracker.generation);
        drop(tracker);
        OperationMetricsScope {
            state: Arc::clone(self),
            generation,
            finished: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn last_report(&self) -> OperationMetricsReport {
        lock_unpoisoned(&self.scope).last_report.clone()
    }

    fn finish_scope(&self, generation: u64, outcome: OperationMetricsOutcome) {
        let mut tracker = lock_unpoisoned(&self.scope);
        if tracker.active.as_ref().map(|active| active.generation) != Some(generation) {
            return;
        }
        tracker.depth = tracker.depth.saturating_sub(1);
        if tracker.depth != 0 {
            return;
        }
        let Some(active) = tracker.active.take() else {
            return;
        };
        let counters_end = self.snapshot();
        let delta = counters_end.saturating_sub(active.counters_start);
        let (rss_end_bytes, rss_lifetime_high_water_bytes) = process_rss_snapshot();
        tracker.last_report = OperationMetricsReport::from_delta(
            active,
            outcome,
            delta,
            counters_end,
            rss_end_bytes,
            rss_lifetime_high_water_bytes,
        );
    }

    pub(crate) fn snapshot(&self) -> OperationMetricsDelta {
        let mut snapshot = self.counters.snapshot();
        let daemon = lock_unpoisoned(&self.daemon_rewrites);
        snapshot.daemon_cumulative_rewrite_count = daemon.count;
        snapshot.daemon_cumulative_rewrite_bytes = daemon.bytes;
        snapshot
    }
}

impl OperationMetricsAccumulator {
    pub(crate) fn new(
        metrics: Option<&Arc<OperationMetricsState>>,
        delta: OperationMetricsDelta,
    ) -> Self {
        Self {
            metrics: metrics.cloned(),
            delta,
        }
    }
}

impl Drop for OperationMetricsAccumulator {
    fn drop(&mut self) {
        if let Some(metrics) = &self.metrics {
            metrics.add(self.delta);
        }
    }
}

pub(crate) fn profile_operation_metrics<T, E>(
    metrics: Option<&Arc<OperationMetricsState>>,
    kind: OperationMetricsKind,
    operation: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    match metrics {
        Some(metrics) => metrics.profile(kind, operation),
        None => operation(),
    }
}

#[cfg(test)]
pub(crate) fn operation_metrics_report(
    metrics: Option<&Arc<OperationMetricsState>>,
) -> Option<OperationMetricsReport> {
    metrics.map(|metrics| metrics.last_report())
}

/// Accepted opt-in values are `1`, `true`, `yes`, and `on`, compared with
/// ASCII case folding and without trimming. Every other value is disabled.
pub(crate) fn operation_metrics_env_value_is_truthy(value: &str) -> bool {
    value == "1"
        || value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("yes")
        || value.eq_ignore_ascii_case("on")
}

pub(crate) fn operation_metrics_are_enabled() -> bool {
    if cfg!(test) {
        return true;
    }
    std::env::var_os(PERFORMANCE_METRICS_ENV)
        .and_then(|value| value.into_string().ok())
        .as_deref()
        .is_some_and(operation_metrics_env_value_is_truthy)
}

impl super::Trail {
    pub(crate) fn note_operation_metrics(&self, delta: OperationMetricsDelta) {
        if let Some(metrics) = &self.operation_metrics {
            metrics.add(delta);
        }
    }
}

impl OperationMetricsScope {
    fn finish(mut self, outcome: OperationMetricsOutcome) {
        self.state.finish_scope(self.generation, outcome);
        self.finished = true;
    }
}

impl Drop for OperationMetricsScope {
    fn drop(&mut self) {
        if !self.finished {
            self.state.finish_scope(
                self.generation,
                OperationMetricsOutcome::CancelledOrUnclassified,
            );
            self.finished = true;
        }
    }
}

impl OperationMetricsReport {
    fn from_delta(
        active: ActiveOperationScope,
        outcome: OperationMetricsOutcome,
        delta: OperationMetricsDelta,
        totals: OperationMetricsDelta,
        rss_end_bytes: u64,
        rss_lifetime_high_water_bytes: u64,
    ) -> Self {
        Self {
            generation: active.generation,
            operation: active.operation.as_str().to_string(),
            outcome,
            input_path_count: delta.input_path_count,
            canonical_path_count: delta.canonical_path_count,
            expanded_path_count: delta.expanded_path_count,
            final_path_count: delta.final_path_count,
            full_filesystem_walk_count: delta.full_filesystem_walk_count,
            bounded_filesystem_walk_count: delta.bounded_filesystem_walk_count,
            filesystem_entry_count: delta.filesystem_entry_count,
            filesystem_stat_count: delta.filesystem_stat_count,
            filesystem_read_count: delta.filesystem_read_count,
            filesystem_read_bytes: delta.filesystem_read_bytes,
            filesystem_hash_count: delta.filesystem_hash_count,
            filesystem_hash_bytes: delta.filesystem_hash_bytes,
            full_root_range_count: delta.full_root_range_count,
            bounded_root_range_count: delta.bounded_root_range_count,
            root_range_row_count: delta.root_range_row_count,
            root_point_key_count: delta.root_point_key_count,
            prolly_read_call_count: delta.prolly_read_call_count,
            prolly_read_key_count: delta.prolly_read_key_count,
            prolly_read_value_count: delta.prolly_read_value_count,
            prolly_read_value_bytes: delta.prolly_read_value_bytes,
            prolly_write_call_count: delta.prolly_write_call_count,
            prolly_write_key_count: delta.prolly_write_key_count,
            prolly_write_value_bytes: delta.prolly_write_value_bytes,
            prolly_tree_batch_call_count: delta.prolly_tree_batch_call_count,
            prolly_tree_batch_mutation_count: delta.prolly_tree_batch_mutation_count,
            selected_worktree_index_sqlite_accounting_complete: delta
                .selected_worktree_index_sqlite_envelope_count
                > 0,
            selected_worktree_index_sqlite_envelope_count: delta
                .selected_worktree_index_sqlite_envelope_count,
            selected_worktree_index_sqlite_full_scan_count: delta
                .selected_worktree_index_sqlite_full_scan_count,
            selected_worktree_index_sqlite_row_read_count: delta
                .selected_worktree_index_sqlite_row_read_count,
            selected_worktree_index_sqlite_row_delete_count: delta
                .selected_worktree_index_sqlite_row_delete_count,
            selected_worktree_index_sqlite_row_upsert_count: delta
                .selected_worktree_index_sqlite_row_upsert_count,
            selected_worktree_index_sqlite_statement_count: delta
                .selected_worktree_index_sqlite_statement_count,
            selected_worktree_index_sqlite_transaction_count: delta
                .selected_worktree_index_sqlite_transaction_count,
            selection_comparison_count: delta.selection_comparison_count,
            policy_build_count: delta.policy_build_count,
            policy_dependency_bytes: delta.policy_dependency_bytes,
            policy_dependency_file_count: delta.policy_dependency_file_count,
            git_subprocess_count: delta.git_subprocess_count,
            git_global_work_count: delta.git_global_work_count,
            git_output_bytes: delta.git_output_bytes,
            git_output_record_count: delta.git_output_record_count,
            daemon_snapshot_bytes: delta.daemon_snapshot_bytes,
            daemon_snapshot_path_count: delta.daemon_snapshot_path_count,
            daemon_cumulative_rewrite_count: delta.daemon_cumulative_rewrite_count,
            daemon_cumulative_rewrite_bytes: delta.daemon_cumulative_rewrite_bytes,
            daemon_cumulative_rewrite_count_total: totals.daemon_cumulative_rewrite_count,
            daemon_cumulative_rewrite_bytes_total: totals.daemon_cumulative_rewrite_bytes,
            authoritative_candidate_count: delta.authoritative_candidate_count,
            ledger_row_touch_count: delta.ledger_row_touch_count,
            observer_tail_record_fold_count: delta.observer_tail_record_fold_count,
            reconciliation_run_count: delta.reconciliation_run_count,
            manifest_bytes: delta.manifest_bytes,
            manifest_key_comparison_count: delta.manifest_key_comparison_count,
            journal_bytes: delta.journal_bytes,
            upper_work_count: delta.upper_work_count,
            wall_time_ns: active.started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            rss_start_bytes: active.rss_start_bytes,
            rss_end_bytes,
            rss_lifetime_high_water_bytes,
        }
    }
}

fn saturating_atomic_add(counter: &AtomicU64, delta: u64) {
    if delta == 0 {
        return;
    }
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(delta))
    });
}

pub(crate) fn saturating_u64_from_usize(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(target_os = "linux")]
fn process_rss_snapshot() -> (u64, u64) {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let page_size = u64::try_from(page_size).unwrap_or(0);
    let current = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|value| value.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages.saturating_mul(page_size))
        .unwrap_or(0);
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let high_water = if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        unsafe { usage.assume_init().ru_maxrss as u64 }.saturating_mul(1024)
    } else {
        0
    };
    (current, high_water.max(current))
}

#[cfg(target_os = "macos")]
fn process_rss_snapshot() -> (u64, u64) {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let high_water = if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        unsafe { usage.assume_init().ru_maxrss as u64 }
    } else {
        0
    };
    // `getrusage` exposes lifetime high-water RSS on macOS, not boundary RSS.
    // Leave the boundary values unknown rather than mislabeling high-water data.
    (0, high_water)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_rss_snapshot() -> (u64, u64) {
    (0, 0)
}
