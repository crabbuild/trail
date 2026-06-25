use super::*;

impl CrabDb {
    pub fn diff_agent(&self, agent: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_agent_with_options(agent, patches, false)
    }

    pub fn diff_agent_with_options(
        &self,
        agent: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let agent_branch = self.agent_branch(agent)?;
        let source = self.get_ref(&agent_branch.ref_name)?;
        let base = self.ref_from_change(&agent_branch.base_change)?;
        self.diff_root_files(
            agent_branch.base_change.0,
            source.change_id.0,
            &base.root_id,
            &source.root_id,
            patches,
            line_changes,
        )
    }
}
