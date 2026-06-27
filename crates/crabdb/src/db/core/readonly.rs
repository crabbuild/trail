use super::*;

impl CrabDb {
    pub(crate) fn enforce_read_only_mcp_call<T, F>(&mut self, label: &str, f: F) -> Result<T>
    where
        F: FnOnce(&mut Self) -> Result<T>,
    {
        let before = self.read_only_fingerprint()?;
        let query_only = self.query_only_enabled()?;
        self.set_query_only(true)?;
        let result = f(self);
        self.set_query_only(query_only)?;
        let after = self.read_only_fingerprint()?;
        if before != after {
            return Err(Error::InvalidInput(format!(
                "read-only MCP call `{label}` attempted to mutate CrabDB state"
            )));
        }
        result
    }

    fn query_only_enabled(&self) -> Result<bool> {
        let enabled = self
            .conn
            .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))?;
        Ok(enabled != 0)
    }

    fn set_query_only(&self, enabled: bool) -> Result<()> {
        let value = if enabled { "ON" } else { "OFF" };
        self.conn
            .execute_batch(&format!("PRAGMA query_only = {value}"))?;
        Ok(())
    }

    fn read_only_fingerprint(&self) -> Result<ReadOnlyFingerprint> {
        let data_version = self
            .conn
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))?;
        let mut stmt = self.conn.prepare(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut tables = Vec::with_capacity(names.len());
        for name in names {
            let quoted = quote_sql_ident(&name);
            let sql = format!("SELECT COUNT(*) FROM {quoted}");
            let count = self.conn.query_row(&sql, [], |row| row.get::<_, i64>(0))?;
            tables.push((name, count));
        }
        Ok(ReadOnlyFingerprint {
            data_version,
            tables,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ReadOnlyFingerprint {
    data_version: i64,
    tables: Vec<(String, i64)>,
}

fn quote_sql_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_tool_guard_blocks_main_connection_writes() {
        let temp = tempfile::tempdir().unwrap();
        CrabDb::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = CrabDb::open(temp.path()).unwrap();

        let err = db
            .enforce_read_only_mcp_call("crabdb.status", |db| {
                db.conn.execute(
                    "INSERT INTO schema_meta (key, value, updated_at) VALUES ('readonly.test', 'x', 0)",
                    [],
                )?;
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(err, Error::Sqlite(_)));
        let stored: Option<String> = db
            .conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'readonly.test'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(stored, None);
    }

    #[test]
    fn read_only_tool_guard_detects_external_connection_writes() {
        let temp = tempfile::tempdir().unwrap();
        CrabDb::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = CrabDb::open(temp.path()).unwrap();

        let err = db
            .enforce_read_only_mcp_call("crabdb.status", |db| {
                let conn = Connection::open(db.db_dir.join(DB_RELATIVE_PATH))?;
                conn.execute(
                    "INSERT INTO prolly_nodes (cid, node) VALUES (x'726561646f6e6c79', x'01')",
                    [],
                )?;
                Ok(())
            })
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(message) if message.contains("attempted to mutate"))
        );
    }
}
