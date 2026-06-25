use super::*;

impl CrabDb {
    pub(crate) fn agent_branch(&self, agent: &str) -> Result<AgentBranch> {
        self.conn
            .query_row(
                "SELECT agent_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at \
                 FROM agent_branches WHERE agent_id = ?1 OR ref_name = ?2 OR agent_id IN (SELECT agent_id FROM agents WHERE name = ?1)",
                params![agent, agent_ref(agent)],
                |row| {
                    Ok(AgentBranch {
                        agent_id: row.get(0)?,
                        ref_name: row.get(1)?,
                        base_change: ChangeId(row.get(2)?),
                        head_change: ChangeId(row.get(3)?),
                        base_root: ObjectId(row.get(4)?),
                        head_root: ObjectId(row.get(5)?),
                        session_id: row.get(6)?,
                        workdir: row.get(7)?,
                        status: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::RefNotFound(agent_ref(agent)))
    }

    pub(crate) fn agent_record(&self, agent_id: &str) -> Result<AgentRecord> {
        self.conn
            .query_row(
                "SELECT agent_id, name, kind, provider, model, created_at, metadata_json \
                 FROM agents WHERE agent_id = ?1 OR name = ?1",
                params![agent_id],
                |row| {
                    Ok(AgentRecord {
                        agent_id: row.get(0)?,
                        name: row.get(1)?,
                        kind: row.get(2)?,
                        provider: row.get(3)?,
                        model: row.get(4)?,
                        created_at: row.get(5)?,
                        metadata_json: row.get(6)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::RefNotFound(agent_id.to_string()))
    }

    pub(crate) fn lease(&self, lease_id: &str) -> Result<LeaseRecord> {
        self.conn
            .query_row(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE lease_id = ?1",
                params![lease_id],
                lease_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("lease `{lease_id}` not found")))
    }

    pub(crate) fn existing_active_lease(
        &self,
        agent_id: &str,
        path: Option<&str>,
        mode: &str,
    ) -> Result<Option<LeaseRecord>> {
        self.conn
            .query_row(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE agent_id = ?1 AND COALESCE(path, '') = COALESCE(?2, '') \
                   AND mode = ?3 AND expires_at > ?4 ORDER BY expires_at DESC LIMIT 1",
                params![agent_id, path, mode, now_ts()],
                lease_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn conflicting_active_leases(
        &self,
        agent_id: &str,
        path: Option<&str>,
        mode: &str,
    ) -> Result<Vec<LeaseRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
             FROM leases WHERE agent_id != ?1 AND COALESCE(path, '') = COALESCE(?2, '') \
               AND expires_at > ?3 ORDER BY expires_at ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![agent_id, path, now_ts()], lease_row)?;
        let leases = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(leases
            .into_iter()
            .filter(|lease| mode == "write" || lease.mode == "write")
            .collect())
    }
}
