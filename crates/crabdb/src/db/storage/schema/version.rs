use super::*;

impl CrabDb {
    pub(crate) fn schema_user_version(&self) -> Result<i64> {
        self.conn
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(Error::from)
    }

    pub(crate) fn set_schema_user_version(&self, version: i64) -> Result<()> {
        self.conn
            .execute_batch(&format!("PRAGMA user_version = {version};"))?;
        Ok(())
    }

    pub(crate) fn record_schema_version(&self) -> Result<()> {
        self.set_schema_user_version(CRABDB_SCHEMA_VERSION)?;
        let now = now_ts();
        for (key, value) in [
            (SCHEMA_META_VERSION_KEY, CRABDB_SCHEMA_VERSION.to_string()),
            (
                SCHEMA_META_APP_VERSION_KEY,
                env!("CARGO_PKG_VERSION").to_string(),
            ),
        ] {
            self.conn.execute(
                "INSERT OR REPLACE INTO schema_meta (key, value, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, now],
            )?;
        }
        Ok(())
    }

    pub(crate) fn schema_meta_value(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }
}
