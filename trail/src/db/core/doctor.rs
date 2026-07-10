use super::*;

impl Trail {
    pub fn doctor(&self) -> Result<DoctorReport> {
        let mut checks = Vec::new();

        doctor_storage::push_workspace_check(self, &mut checks);
        doctor_storage::push_database_check(self, &mut checks);
        doctor_storage::push_schema_check(self, &mut checks);
        doctor_storage::push_current_branch_check(self, &mut checks);
        doctor_storage::push_ignore_policy_check(self, &mut checks);
        doctor_storage::push_workspace_views_check(self, &mut checks);

        doctor_runtime::push_write_lock_check(self, &mut checks);
        doctor_runtime::push_daemon_token_check(self, &mut checks);
        doctor_runtime::push_fsck_check(self, &mut checks);

        doctor_activity::push_pending_approvals_check(self, &mut checks);
        doctor_activity::push_active_leases_check(self, &mut checks);
        doctor_activity::push_merge_queue_check(self, &mut checks);
        doctor_activity::push_conflicts_check(self, &mut checks);
        doctor_activity::push_lanes_check(self, &mut checks);

        Ok(doctor_report(checks))
    }
}
