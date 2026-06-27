use super::*;

impl CrabDb {
    pub fn diff_lane(&self, lane: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_lane_with_options(lane, patches, false)
    }

    pub fn diff_lane_with_options(
        &self,
        lane: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let lane_branch = self.lane_branch(lane)?;
        let source = self.get_ref(&lane_branch.ref_name)?;
        let base = self.ref_from_change(&lane_branch.base_change)?;
        self.diff_root_files(
            lane_branch.base_change.0,
            source.change_id.0,
            &base.root_id,
            &source.root_id,
            patches,
            line_changes,
        )
    }
}
