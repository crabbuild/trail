use super::*;

impl Trail {
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
                "read-only MCP call `{label}` attempted to mutate Trail state"
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
        let files = self.read_only_file_fingerprint()?;
        Ok(ReadOnlyFingerprint {
            data_version,
            tables,
            files,
        })
    }

    fn read_only_file_fingerprint(&self) -> Result<Vec<ReadOnlyFileFingerprint>> {
        let mut files = Vec::new();
        for rel in [
            CONFIG_FILE,
            HEAD_FILE,
            "refs",
            "daemon.json",
            "daemon.token",
        ] {
            collect_read_only_file_fingerprints(
                &self.db_dir.join(rel),
                &format!("db:{rel}"),
                &mut files,
            )?;
        }

        let mut stmt = self.conn.prepare(
            "SELECT workdir FROM lane_branches WHERE workdir IS NOT NULL ORDER BY workdir",
        )?;
        let workdirs = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        for workdir in workdirs {
            let metadata_dir = PathBuf::from(&workdir).join(".trail");
            collect_read_only_file_fingerprints(
                &metadata_dir,
                &format!("workdir:{}/.trail", workdir),
                &mut files,
            )?;
        }

        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ReadOnlyFingerprint {
    data_version: i64,
    tables: Vec<(String, i64)>,
    files: Vec<ReadOnlyFileFingerprint>,
}

#[derive(Debug, PartialEq, Eq)]
struct ReadOnlyFileFingerprint {
    path: String,
    kind: &'static str,
    len: Option<u64>,
    hash: Option<String>,
    symlink_target: Option<String>,
}

fn quote_sql_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn collect_read_only_file_fingerprints(
    path: &Path,
    label: &str,
    out: &mut Vec<ReadOnlyFileFingerprint>,
) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Io(err)),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        out.push(ReadOnlyFileFingerprint {
            path: label.to_string(),
            kind: "dir",
            len: None,
            hash: None,
            symlink_target: None,
        });
        let mut entries = fs::read_dir(path)?.collect::<std::result::Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            collect_read_only_file_fingerprints(&entry.path(), &format!("{label}/{name}"), out)?;
        }
        return Ok(());
    }

    if metadata.file_type().is_symlink() {
        out.push(ReadOnlyFileFingerprint {
            path: label.to_string(),
            kind: "symlink",
            len: None,
            hash: None,
            symlink_target: Some(fs::read_link(path)?.to_string_lossy().to_string()),
        });
        return Ok(());
    }

    if metadata.is_file() {
        let bytes = fs::read(path)?;
        out.push(ReadOnlyFileFingerprint {
            path: label.to_string(),
            kind: "file",
            len: Some(metadata.len()),
            hash: Some(sha256_hex(&bytes)),
            symlink_target: None,
        });
        return Ok(());
    }

    out.push(ReadOnlyFileFingerprint {
        path: label.to_string(),
        kind: "other",
        len: Some(metadata.len()),
        hash: None,
        symlink_target: None,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_tool_guard_blocks_main_connection_writes() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();

        let err = db
            .enforce_read_only_mcp_call("trail.status", |db| {
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
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();

        let err = db
            .enforce_read_only_mcp_call("trail.status", |db| {
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

    #[test]
    fn read_only_tool_guard_detects_config_sidecar_writes() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();

        let err = db
            .enforce_read_only_mcp_call("trail.status", |db| {
                fs::write(db.db_dir.join(CONFIG_FILE), b"mutated = true\n")?;
                Ok(())
            })
            .unwrap_err();

        assert!(
            matches!(err, Error::InvalidInput(message) if message.contains("attempted to mutate"))
        );
    }

    #[test]
    fn read_only_tool_guard_detects_ref_sidecar_writes() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();

        let err = db
            .enforce_read_only_mcp_call("trail.status", |db| {
                fs::write(db.db_dir.join("refs/branches/sidecar"), b"mutated\n")?;
                Ok(())
            })
            .unwrap_err();

        assert!(
            matches!(err, Error::InvalidInput(message) if message.contains("attempted to mutate"))
        );
    }

    #[test]
    fn read_only_tool_guard_detects_lane_workdir_metadata_writes() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let spawned = db
            .spawn_lane("readonly-meta-bot", Some("main"), true, None, None)
            .unwrap();
        let workdir_metadata = PathBuf::from(spawned.workdir.unwrap()).join(".trail");

        let err = db
            .enforce_read_only_mcp_call("trail.status", |_| {
                fs::write(
                    workdir_metadata.join("workdir-manifest.json"),
                    b"{\"mutated\":true}\n",
                )?;
                Ok(())
            })
            .unwrap_err();

        assert!(
            matches!(err, Error::InvalidInput(message) if message.contains("attempted to mutate"))
        );
    }
}
