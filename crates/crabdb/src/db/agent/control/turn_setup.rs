use super::*;

impl CrabDb {
    pub(crate) fn agent_branch_for_turn(
        &mut self,
        agent: &str,
        from: Option<&str>,
        base_change: Option<&str>,
    ) -> Result<AgentBranch> {
        match self.agent_branch(agent) {
            Ok(branch) => Ok(branch),
            Err(Error::RefNotFound(_)) => {
                self.spawn_agent_branch_for_turn(agent, from, base_change)
            }
            Err(err) => Err(err),
        }
    }

    fn spawn_agent_branch_for_turn(
        &mut self,
        agent: &str,
        from: Option<&str>,
        base_change: Option<&str>,
    ) -> Result<AgentBranch> {
        let source_selector = match base_change.or(from) {
            Some(selector) => selector.to_string(),
            None => self.current_branch()?,
        };
        let source = self.resolve_refish(&source_selector)?;
        let agent_id = format!("agent_{}", crate::ids::short_hash(agent.as_bytes(), 8));
        let ref_name = agent_ref(agent);
        if self.try_get_ref(&ref_name)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` already exists"
            )));
        }
        let workdir = if self.default_agent_materialize_for_ref(Some(&source_selector))? {
            let dir = self.materialize_agent_workdir(agent, &source.root_id, None)?;
            Some(dir.to_string_lossy().to_string())
        } else {
            None
        };
        self.set_ref(
            &ref_name,
            &source.change_id,
            &source.root_id,
            &source.operation_id,
        )?;
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agents (agent_id, name, kind, provider, model, created_at, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                agent_id,
                agent,
                "coding-agent",
                Option::<String>::None,
                Option::<String>::None,
                now,
                Option::<String>::None
            ],
        )?;
        self.conn.execute(
            "INSERT INTO agent_branches \
             (agent_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, 'active', ?8, ?8)",
            params![
                agent_id,
                ref_name,
                source.change_id.0,
                source.change_id.0,
                source.root_id.0,
                source.root_id.0,
                workdir.clone(),
                now
            ],
        )?;
        self.insert_agent_event(
            &format!("agent_{}", crate::ids::short_hash(agent.as_bytes(), 8)),
            "agent_spawned",
            Some(&source.change_id),
            None,
            &serde_json::json!({
                "ref_name": agent_ref(agent),
                "base_root": source.root_id.0.clone(),
                "workdir": workdir.clone(),
                "source": "api"
            }),
        )?;
        self.agent_branch(agent)
    }
}
