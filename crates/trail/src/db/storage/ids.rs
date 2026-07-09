use super::*;

impl Trail {
    pub(crate) fn allocate_change_id(&self, actor_id: &str, hint: &str) -> Result<ChangeId> {
        let lamport = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(generation), 0) + 1 FROM refs",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1);
        Ok(ChangeId::allocate(
            &self.config.workspace.id,
            actor_id,
            lamport,
            hint,
        ))
    }
}
