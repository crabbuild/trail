use super::*;
use rusqlite::types::Value as SqlValue;

const SQLITE_VEC_BACKEND: &str = "sqlite_vec0";

impl Trail {
    pub fn put_memory(&self, input: MemoryPut) -> Result<MemoryItem> {
        let scope_type = required_text("memory scope_type", &input.scope_type)?;
        let scope_id = required_text("memory scope_id", &input.scope_id)?;
        let kind = required_text("memory kind", &input.kind)?;
        let actor_id = required_text("memory actor_id", &input.actor_id)?;
        let body = redact_sensitive_text(&required_text("memory body", &input.body)?);
        let title =
            optional_text(input.title.as_deref()).map(|title| redact_sensitive_text(&title));
        let path = optional_text(input.path.as_deref());
        let source_ref = optional_text(input.source.source_ref.as_deref());
        let source_change = input.source.source_change.as_ref().map(|id| id.0.clone());
        let source_root = input.source.source_root.as_ref().map(|id| id.0.clone());
        let metadata = redact_sensitive_json(input.metadata);
        let metadata_json = serde_json::to_string(&metadata)?;
        let embedding = input.embedding.map(normalize_embedding_input).transpose()?;
        let now = now_ts();

        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let result = (|| {
            let existing = input
                .memory_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|memory_id| self.memory_ord(memory_id))
                .transpose()?
                .flatten();

            let (memory_id, memory_ord, operation) = if let Some((memory_id, memory_ord)) = existing
            {
                self.conn.execute(
                    "UPDATE memory_items SET \
                     scope_type = ?1, scope_id = ?2, kind = ?3, path = ?4, title = ?5, body = ?6, \
                     status = 'active', source_ref = ?7, source_change = ?8, source_root = ?9, \
                     metadata_json = ?10, updated_by = ?11, updated_at = ?12, archived_at = NULL \
                     WHERE memory_id = ?13",
                    params![
                        scope_type,
                        scope_id,
                        kind,
                        path,
                        title,
                        body,
                        source_ref,
                        source_change,
                        source_root,
                        metadata_json,
                        actor_id,
                        now,
                        memory_id
                    ],
                )?;
                (memory_id, memory_ord, "update")
            } else {
                let memory_id = input
                    .memory_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| allocate_memory_id(&scope_type, &scope_id, &kind, &body));
                self.conn.execute(
                    "INSERT INTO memory_items \
                     (memory_id, scope_type, scope_id, kind, path, title, body, status, source_ref, source_change, source_root, metadata_json, created_by, updated_by, created_at, updated_at, archived_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', ?8, ?9, ?10, ?11, ?12, ?12, ?13, ?13, NULL)",
                    params![
                        memory_id,
                        scope_type,
                        scope_id,
                        kind,
                        path,
                        title,
                        body,
                        source_ref,
                        source_change,
                        source_root,
                        metadata_json,
                        actor_id,
                        now
                    ],
                )?;
                let memory_ord = self.conn.last_insert_rowid();
                (memory_id, memory_ord, "put")
            };

            let embedding_hash = if let Some(embedding) = embedding.as_ref() {
                self.put_memory_embedding(&memory_id, memory_ord, embedding, now)?;
                Some(embedding.embedding_hash.clone())
            } else {
                self.remove_memory_embedding(&memory_id, memory_ord)?;
                None
            };

            self.record_memory_revision(
                &memory_id,
                operation,
                embedding_hash.as_deref(),
                &actor_id,
            )?;
            self.memory_item(&memory_id)
        })();

        if result.is_ok() {
            self.conn.execute_batch("COMMIT;")?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK;");
        }
        result
    }

    pub fn archive_memory(
        &self,
        memory_id: &str,
        actor_id: &str,
        source: MemoryVersionSource,
    ) -> Result<MemoryItem> {
        let memory_id = required_text("memory_id", memory_id)?;
        let actor_id = required_text("memory actor_id", actor_id)?;
        let source_ref = optional_text(source.source_ref.as_deref());
        let source_change = source.source_change.as_ref().map(|id| id.0.clone());
        let source_root = source.source_root.as_ref().map(|id| id.0.clone());
        let now = now_ts();

        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let result = (|| {
            let (_, memory_ord) = self
                .memory_ord(&memory_id)?
                .ok_or_else(|| Error::InvalidInput(format!("memory `{memory_id}` not found")))?;
            let changed = self.conn.execute(
                "UPDATE memory_items SET \
                 status = 'archived', source_ref = ?1, source_change = ?2, source_root = ?3, \
                 updated_by = ?4, updated_at = ?5, archived_at = ?5 \
                 WHERE memory_id = ?6",
                params![
                    source_ref,
                    source_change,
                    source_root,
                    actor_id,
                    now,
                    memory_id
                ],
            )?;
            if changed == 0 {
                return Err(Error::InvalidInput(format!(
                    "memory `{memory_id}` not found"
                )));
            }
            self.refresh_memory_vec_row(&memory_id, memory_ord)?;
            self.record_memory_revision(&memory_id, "archive", None, &actor_id)?;
            self.memory_item(&memory_id)
        })();

        if result.is_ok() {
            self.conn.execute_batch("COMMIT;")?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK;");
        }
        result
    }

    pub fn memory_item(&self, memory_id: &str) -> Result<MemoryItem> {
        let memory_id = required_text("memory_id", memory_id)?;
        self.conn
            .query_row(
                memory_item_select_sql("WHERE i.memory_id = ?1").as_str(),
                params![memory_id],
                memory_item_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("memory `{memory_id}` not found")))
    }

    pub fn search_memory(&self, query: MemorySearch) -> Result<Vec<MemorySearchResult>> {
        let top_k = normalized_top_k(query.top_k);
        if let Some(embedding) = query.query_embedding.as_ref() {
            validate_embedding_vector("query embedding", embedding)?;
            if query.backend != MemorySearchBackend::Exact {
                if let Some(results) = self.search_memory_with_vec0(&query, embedding, top_k)? {
                    return Ok(results);
                }
                if query.backend == MemorySearchBackend::SqliteVec {
                    return Err(Error::InvalidInput(
                        "sqlite_vec memory search requires an existing provider/model/dims index"
                            .to_string(),
                    ));
                }
            }
            self.search_memory_exact(&query, embedding, top_k)
        } else {
            self.list_memory(query, top_k)
        }
    }

    pub fn memory_context_packet(&self, query: MemorySearch) -> Result<MemoryContextPacket> {
        let backend = query.backend;
        let results = self.search_memory(query)?;
        let entries = results
            .into_iter()
            .map(|result| MemoryContextEntry {
                citation: memory_citation(&result.item),
                memory_id: result.item.memory_id,
                title: result.item.title,
                path: result.item.path,
                body: result.item.body,
                distance: result.distance,
            })
            .collect();
        Ok(MemoryContextPacket { backend, entries })
    }

    pub fn memory_revisions(&self, memory_id: &str) -> Result<Vec<MemoryRevision>> {
        let memory_id = required_text("memory_id", memory_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT revision_id, memory_id, version, operation, scope_type, scope_id, kind, path, title, body, status, \
             source_ref, source_change, source_root, metadata_json, embedding_hash, actor_id, created_at \
             FROM memory_revisions WHERE memory_id = ?1 ORDER BY version ASC",
        )?;
        let rows = stmt.query_map(params![memory_id], memory_revision_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn memory_revision(&self, memory_id: &str, version: i64) -> Result<MemoryRevision> {
        let memory_id = required_text("memory_id", memory_id)?;
        if version <= 0 {
            return Err(Error::InvalidInput(
                "memory version must be positive".to_string(),
            ));
        }
        self.conn
            .query_row(
                "SELECT revision_id, memory_id, version, operation, scope_type, scope_id, kind, path, title, body, status, \
                 source_ref, source_change, source_root, metadata_json, embedding_hash, actor_id, created_at \
                 FROM memory_revisions WHERE memory_id = ?1 AND version = ?2",
                params![memory_id, version],
                memory_revision_row,
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!("memory `{memory_id}` version {version} not found"))
            })
    }

    fn list_memory(&self, query: MemorySearch, top_k: usize) -> Result<Vec<MemorySearchResult>> {
        let (where_sql, values) = memory_filter_sql(&query, None);
        let sql = format!(
            "{} ORDER BY i.updated_at DESC, i.memory_id ASC LIMIT ?",
            memory_item_select_sql(&where_sql)
        );
        let mut values = values;
        values.push(SqlValue::Integer(top_k as i64));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(values.iter()), memory_item_row)?;
        rows.map(|row| {
            row.map(|item| MemorySearchResult {
                item,
                distance: None,
            })
        })
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Error::from)
    }

    fn search_memory_exact(
        &self,
        query: &MemorySearch,
        embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<MemorySearchResult>> {
        let (where_sql, mut values) = memory_filter_sql(query, Some(embedding.len()));
        let sql = format!(
            "{} AND e.embedding IS NOT NULL ORDER BY i.updated_at DESC, i.memory_id ASC",
            memory_item_select_sql(&where_sql)
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(values.iter()), |row| {
            Ok((memory_item_row(row)?, row.get::<_, Option<Vec<u8>>>(22)?))
        })?;
        let mut results = Vec::new();
        for row in rows {
            let (item, bytes) = row?;
            let Some(bytes) = bytes else {
                continue;
            };
            let stored = decode_f32_vec(&bytes)?;
            if stored.len() == embedding.len() {
                results.push(MemorySearchResult {
                    item,
                    distance: Some(cosine_distance(embedding, &stored)),
                });
            }
        }
        results.sort_by(compare_memory_results);
        results.truncate(top_k);
        values.clear();
        Ok(results)
    }

    fn search_memory_with_vec0(
        &self,
        query: &MemorySearch,
        embedding: &[f32],
        top_k: usize,
    ) -> Result<Option<Vec<MemorySearchResult>>> {
        let Some(provider) = query
            .embedding_provider
            .as_deref()
            .and_then(non_empty_trimmed)
        else {
            return Ok(None);
        };
        let Some(model) = query.embedding_model.as_deref().and_then(non_empty_trimmed) else {
            return Ok(None);
        };
        if query.source_ref.is_some() || query.source_change.is_some() {
            return Ok(None);
        }
        if query.backend == MemorySearchBackend::Auto
            && query.path_prefix.is_some()
            && query.scope_type.is_none()
            && query.scope_id.is_none()
        {
            return Ok(None);
        }
        let Some(table_name) = self.memory_vec_table(provider, model, embedding.len())? else {
            return Ok(None);
        };

        let (where_sql, mut values) = memory_filter_sql(query, None);
        let mut vec_where = String::from("WHERE embedding MATCH ? AND k = ?");
        let mut vec_values = vec![
            SqlValue::Blob(encode_f32_vec(embedding)),
            SqlValue::Integer(top_k as i64),
        ];
        append_vec0_filters(query, &mut vec_where, &mut vec_values);
        let sql = format!(
            "WITH matches AS (\
                SELECT memory_ord, distance FROM {table_name} {vec_where}\
             ) \
             {} ORDER BY matches.distance, i.memory_id ASC",
            memory_item_select_sql_extra(
                "JOIN matches ON matches.memory_ord = i.memory_ord",
                &where_sql,
                ", matches.distance"
            )
        );
        vec_values.append(&mut values);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(vec_values.iter()), |row| {
            let item = memory_item_row(row)?;
            let distance = row.get::<_, Option<f32>>(23)?;
            Ok(MemorySearchResult { item, distance })
        })?;
        let mut results = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        results.sort_by(compare_memory_results);
        results.truncate(top_k);
        Ok(Some(results))
    }

    fn put_memory_embedding(
        &self,
        memory_id: &str,
        memory_ord: i64,
        embedding: &NormalizedMemoryEmbedding,
        updated_at: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO memory_embeddings \
             (memory_id, memory_ord, provider, model, dims, embedding, embedding_hash, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(memory_id) DO UPDATE SET \
                memory_ord = excluded.memory_ord, provider = excluded.provider, model = excluded.model, \
                dims = excluded.dims, embedding = excluded.embedding, embedding_hash = excluded.embedding_hash, updated_at = excluded.updated_at",
            params![
                memory_id,
                memory_ord,
                embedding.provider,
                embedding.model,
                embedding.dims as i64,
                embedding.bytes,
                embedding.embedding_hash,
                updated_at
            ],
        )?;
        self.refresh_memory_vec_row(memory_id, memory_ord)
    }

    fn remove_memory_embedding(&self, memory_id: &str, memory_ord: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM memory_embeddings WHERE memory_id = ?1",
            params![memory_id],
        )?;
        self.remove_memory_from_vec_indexes(memory_ord)
    }

    fn remove_memory_from_vec_indexes(&self, memory_ord: i64) -> Result<()> {
        let tables = self.memory_vec_tables()?;
        for table in tables {
            self.conn.execute(
                &format!("DELETE FROM {table} WHERE memory_ord = ?1"),
                params![memory_ord],
            )?;
        }
        Ok(())
    }

    fn refresh_memory_vec_row(&self, memory_id: &str, memory_ord: i64) -> Result<()> {
        self.remove_memory_from_vec_indexes(memory_ord)?;
        let embedding = self
            .conn
            .query_row(
                "SELECT provider, model, dims, embedding FROM memory_embeddings WHERE memory_id = ?1",
                params![memory_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? as usize,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((provider, model, dims, bytes)) = embedding else {
            return Ok(());
        };
        let item = self.memory_item(memory_id)?;
        let table_name = self.ensure_memory_vec_table(&provider, &model, dims)?;
        self.conn.execute(
            &format!(
                "INSERT INTO {table_name} (memory_ord, embedding, scope_type, scope_id, kind, path, status) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            ),
            params![
                memory_ord,
                bytes,
                item.scope_type,
                item.scope_id,
                item.kind,
                item.path,
                item.status
            ],
        )?;
        Ok(())
    }

    fn ensure_memory_vec_table(&self, provider: &str, model: &str, dims: usize) -> Result<String> {
        let index_id = memory_vec_index_id(provider, model, dims);
        let table_name = memory_vec_table_name(&index_id);
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO memory_embedding_indexes \
             (index_id, backend, provider, model, dims, table_name, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) \
             ON CONFLICT(backend, provider, model, dims) DO UPDATE SET updated_at = excluded.updated_at",
            params![
                index_id,
                SQLITE_VEC_BACKEND,
                provider,
                model,
                dims as i64,
                table_name,
                now
            ],
        )?;
        self.conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {table_name} USING vec0(\
                memory_ord INTEGER PRIMARY KEY,\
                embedding float[{dims}] distance_metric=cosine,\
                scope_type TEXT,\
                scope_id TEXT,\
                kind TEXT,\
                path TEXT,\
                status TEXT\
            );"
        ))?;
        Ok(table_name)
    }

    fn memory_vec_table(&self, provider: &str, model: &str, dims: usize) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT table_name FROM memory_embedding_indexes \
                 WHERE backend = ?1 AND provider = ?2 AND model = ?3 AND dims = ?4",
                params![SQLITE_VEC_BACKEND, provider, model, dims as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    fn memory_vec_tables(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT table_name FROM memory_embedding_indexes WHERE backend = ?1 ORDER BY table_name",
        )?;
        let rows = stmt.query_map(params![SQLITE_VEC_BACKEND], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn memory_ord(&self, memory_id: &str) -> Result<Option<(String, i64)>> {
        self.conn
            .query_row(
                "SELECT memory_id, memory_ord FROM memory_items WHERE memory_id = ?1",
                params![memory_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(Error::from)
    }

    fn record_memory_revision(
        &self,
        memory_id: &str,
        operation: &str,
        embedding_hash: Option<&str>,
        actor_id: &str,
    ) -> Result<()> {
        let version = self.next_memory_version(memory_id)?;
        let item = self.memory_item(memory_id)?;
        let revision_embedding_hash = embedding_hash.or_else(|| {
            item.embedding
                .as_ref()
                .map(|embedding| embedding.embedding_hash.as_str())
        });
        let created_at = now_ts();
        let revision_id = format!(
            "memrev_{}",
            crate::ids::short_hash(
                format!("{memory_id}:{version}:{operation}:{created_at}").as_bytes(),
                16
            )
        );
        self.conn.execute(
            "INSERT INTO memory_revisions \
             (revision_id, memory_id, version, operation, scope_type, scope_id, kind, path, title, body, status, \
              source_ref, source_change, source_root, metadata_json, embedding_hash, actor_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                revision_id,
                memory_id,
                version,
                operation,
                item.scope_type,
                item.scope_id,
                item.kind,
                item.path,
                item.title,
                item.body,
                item.status,
                item.source.source_ref,
                item.source.source_change.map(|id| id.0),
                item.source.source_root.map(|id| id.0),
                serde_json::to_string(&item.metadata)?,
                revision_embedding_hash,
                actor_id,
                created_at
            ],
        )?;
        Ok(())
    }

    fn next_memory_version(&self, memory_id: &str) -> Result<i64> {
        let version: Option<i64> = self.conn.query_row(
            "SELECT MAX(version) FROM memory_revisions WHERE memory_id = ?1",
            params![memory_id],
            |row| row.get(0),
        )?;
        Ok(version.unwrap_or(0) + 1)
    }
}

#[derive(Debug)]
struct NormalizedMemoryEmbedding {
    provider: String,
    model: String,
    dims: usize,
    bytes: Vec<u8>,
    embedding_hash: String,
}

fn normalize_embedding_input(input: MemoryEmbeddingInput) -> Result<NormalizedMemoryEmbedding> {
    let provider = required_text("embedding provider", &input.provider)?;
    let model = required_text("embedding model", &input.model)?;
    validate_embedding_vector("embedding", &input.vector)?;
    let dims = input.vector.len();
    let bytes = encode_f32_vec(&input.vector);
    let embedding_hash = sha256_hex(&bytes);
    Ok(NormalizedMemoryEmbedding {
        provider,
        model,
        dims,
        bytes,
        embedding_hash,
    })
}

fn required_text(label: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::InvalidInput(format!("{label} cannot be empty")));
    }
    Ok(value.to_string())
}

fn optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(non_empty_trimmed).map(str::to_string)
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn normalized_top_k(top_k: usize) -> usize {
    top_k.clamp(1, 200)
}

fn validate_embedding_vector(label: &str, vector: &[f32]) -> Result<()> {
    if vector.is_empty() {
        return Err(Error::InvalidInput(format!("{label} cannot be empty")));
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(Error::InvalidInput(format!(
            "{label} must contain only finite values"
        )));
    }
    Ok(())
}

fn allocate_memory_id(scope_type: &str, scope_id: &str, kind: &str, body: &str) -> String {
    format!(
        "mem_{}",
        crate::ids::short_hash(
            format!("{scope_type}:{scope_id}:{kind}:{body}:{}", now_nanos()).as_bytes(),
            16
        )
    )
}

fn memory_item_select_sql(where_sql: &str) -> String {
    memory_item_select_sql_extra("", where_sql, "")
}

fn memory_item_select_sql_extra(join_sql: &str, where_sql: &str, extra_select: &str) -> String {
    format!(
        "SELECT i.memory_id, i.scope_type, i.scope_id, i.kind, i.path, i.title, i.body, i.status, \
         i.source_ref, i.source_change, i.source_root, i.metadata_json, i.created_by, i.updated_by, \
         i.created_at, i.updated_at, i.archived_at, e.provider, e.model, e.dims, e.embedding_hash, e.updated_at, e.embedding{extra_select} \
         FROM memory_items i LEFT JOIN memory_embeddings e ON e.memory_id = i.memory_id {join_sql} {where_sql}"
    )
}

fn memory_item_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryItem> {
    let metadata_json: String = row.get(11)?;
    let source_change = row.get::<_, Option<String>>(9)?.map(ChangeId);
    let source_root = row.get::<_, Option<String>>(10)?.map(ObjectId);
    let provider = row.get::<_, Option<String>>(17)?;
    let embedding = if let Some(provider) = provider {
        Some(MemoryEmbeddingInfo {
            provider,
            model: row.get(18)?,
            dims: row.get::<_, i64>(19)? as usize,
            embedding_hash: row.get(20)?,
            updated_at: row.get(21)?,
        })
    } else {
        None
    };
    Ok(MemoryItem {
        memory_id: row.get(0)?,
        scope_type: row.get(1)?,
        scope_id: row.get(2)?,
        kind: row.get(3)?,
        path: row.get(4)?,
        title: row.get(5)?,
        body: row.get(6)?,
        status: row.get(7)?,
        source: MemoryVersionSource {
            source_ref: row.get(8)?,
            source_change,
            source_root,
        },
        metadata: serde_json::from_str(&metadata_json).unwrap_or_else(|_| serde_json::json!({})),
        created_by: row.get(12)?,
        updated_by: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        archived_at: row.get(16)?,
        embedding,
    })
}

fn memory_revision_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRevision> {
    let metadata_json: String = row.get(14)?;
    Ok(MemoryRevision {
        revision_id: row.get(0)?,
        memory_id: row.get(1)?,
        version: row.get(2)?,
        operation: row.get(3)?,
        scope_type: row.get(4)?,
        scope_id: row.get(5)?,
        kind: row.get(6)?,
        path: row.get(7)?,
        title: row.get(8)?,
        body: row.get(9)?,
        status: row.get(10)?,
        source: MemoryVersionSource {
            source_ref: row.get(11)?,
            source_change: row.get::<_, Option<String>>(12)?.map(ChangeId),
            source_root: row.get::<_, Option<String>>(13)?.map(ObjectId),
        },
        metadata: serde_json::from_str(&metadata_json).unwrap_or_else(|_| serde_json::json!({})),
        embedding_hash: row.get(15)?,
        actor_id: row.get(16)?,
        created_at: row.get(17)?,
    })
}

fn memory_filter_sql(
    query: &MemorySearch,
    required_dims: Option<usize>,
) -> (String, Vec<SqlValue>) {
    let mut where_sql = String::from("WHERE 1 = 1");
    let mut values = Vec::new();
    push_filter(
        &mut where_sql,
        &mut values,
        "i.scope_type",
        &query.scope_type,
    );
    push_filter(&mut where_sql, &mut values, "i.scope_id", &query.scope_id);
    push_filter(&mut where_sql, &mut values, "i.kind", &query.kind);
    push_filter(
        &mut where_sql,
        &mut values,
        "i.source_ref",
        &query.source_ref,
    );
    if let Some(change) = &query.source_change {
        where_sql.push_str(" AND i.source_change = ?");
        values.push(SqlValue::Text(change.0.clone()));
    }
    if let Some(status) = query.status.as_deref().and_then(non_empty_trimmed) {
        where_sql.push_str(" AND i.status = ?");
        values.push(SqlValue::Text(status.to_string()));
    }
    if let Some(prefix) = query.path_prefix.as_deref().and_then(non_empty_trimmed) {
        where_sql.push_str(" AND i.path >= ? AND i.path < ?");
        values.push(SqlValue::Text(prefix.to_string()));
        values.push(SqlValue::Text(prefix_upper_bound(prefix)));
    }
    if let Some(provider) = query
        .embedding_provider
        .as_deref()
        .and_then(non_empty_trimmed)
    {
        where_sql.push_str(" AND e.provider = ?");
        values.push(SqlValue::Text(provider.to_string()));
    }
    if let Some(model) = query.embedding_model.as_deref().and_then(non_empty_trimmed) {
        where_sql.push_str(" AND e.model = ?");
        values.push(SqlValue::Text(model.to_string()));
    }
    if let Some(dims) = required_dims {
        where_sql.push_str(" AND e.dims = ?");
        values.push(SqlValue::Integer(dims as i64));
    }
    (where_sql, values)
}

fn push_filter(sql: &mut String, values: &mut Vec<SqlValue>, column: &str, value: &Option<String>) {
    if let Some(value) = value.as_deref().and_then(non_empty_trimmed) {
        sql.push_str(" AND ");
        sql.push_str(column);
        sql.push_str(" = ?");
        values.push(SqlValue::Text(value.to_string()));
    }
}

fn append_vec0_filters(query: &MemorySearch, sql: &mut String, values: &mut Vec<SqlValue>) {
    push_vec0_filter(sql, values, "scope_type", &query.scope_type);
    push_vec0_filter(sql, values, "scope_id", &query.scope_id);
    push_vec0_filter(sql, values, "kind", &query.kind);
    if let Some(status) = query.status.as_deref().and_then(non_empty_trimmed) {
        sql.push_str(" AND status = ?");
        values.push(SqlValue::Text(status.to_string()));
    }
    if let Some(prefix) = query.path_prefix.as_deref().and_then(non_empty_trimmed) {
        sql.push_str(" AND path >= ? AND path < ?");
        values.push(SqlValue::Text(prefix.to_string()));
        values.push(SqlValue::Text(prefix_upper_bound(prefix)));
    }
}

fn push_vec0_filter(
    sql: &mut String,
    values: &mut Vec<SqlValue>,
    column: &str,
    value: &Option<String>,
) {
    if let Some(value) = value.as_deref().and_then(non_empty_trimmed) {
        sql.push_str(" AND ");
        sql.push_str(column);
        sql.push_str(" = ?");
        values.push(SqlValue::Text(value.to_string()));
    }
}

fn memory_vec_index_id(provider: &str, model: &str, dims: usize) -> String {
    format!(
        "memidx_{}",
        crate::ids::short_hash(format!("{provider}:{model}:{dims}").as_bytes(), 16)
    )
}

fn memory_vec_table_name(index_id: &str) -> String {
    format!("memory_vec_{}", index_id.trim_start_matches("memidx_"))
}

fn memory_citation(item: &MemoryItem) -> String {
    match (&item.path, &item.source.source_change) {
        (Some(path), Some(change)) => format!("memory:{}:{}:{}", item.memory_id, path, change.0),
        (Some(path), None) => format!("memory:{}:{}", item.memory_id, path),
        (None, Some(change)) => format!("memory:{}:{}", item.memory_id, change.0),
        (None, None) => format!("memory:{}", item.memory_id),
    }
}

fn compare_memory_results(
    left: &MemorySearchResult,
    right: &MemorySearchResult,
) -> std::cmp::Ordering {
    let distance_order = match (left.distance, right.distance) {
        (Some(left), Some(right)) => left
            .partial_cmp(&right)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    };
    distance_order
        .then_with(|| right.item.updated_at.cmp(&left.item.updated_at))
        .then_with(|| left.item.memory_id.cmp(&right.item.memory_id))
}

fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return 1.0;
    }
    1.0 - dot / (left_norm.sqrt() * right_norm.sqrt())
}

fn encode_f32_vec(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(vector));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_f32_vec(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(std::mem::size_of::<f32>()) {
        return Err(Error::Corrupt(
            "stored memory embedding has invalid byte length".to_string(),
        ));
    }
    Ok(bytes
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

fn prefix_upper_bound(prefix: &str) -> String {
    let mut bytes = prefix.as_bytes().to_vec();
    for index in (0..bytes.len()).rev() {
        if bytes[index] != u8::MAX {
            bytes[index] += 1;
            bytes.truncate(index + 1);
            return String::from_utf8(bytes).unwrap();
        }
    }
    format!("{prefix}\u{10ffff}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_db() -> (tempfile::TempDir, Trail) {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        (temp, db)
    }

    fn embedding(values: &[f32]) -> MemoryEmbeddingInput {
        MemoryEmbeddingInput {
            provider: "test".to_string(),
            model: "tiny".to_string(),
            vector: values.to_vec(),
        }
    }

    fn put(scope_id: &str, body: &str, vector: &[f32]) -> MemoryPut {
        MemoryPut {
            memory_id: None,
            scope_type: "lane".to_string(),
            scope_id: scope_id.to_string(),
            kind: "decision".to_string(),
            path: Some("src/memory.rs".to_string()),
            title: Some(body.to_string()),
            body: body.to_string(),
            actor_id: "agent:test".to_string(),
            source: MemoryVersionSource::default(),
            metadata: serde_json::json!({"test": true}),
            embedding: Some(embedding(vector)),
        }
    }

    #[test]
    fn memory_put_search_and_revisions_round_trip() {
        let (_temp, db) = empty_db();
        let first = db
            .put_memory(put(
                "lane-1",
                "use sqlite for durable memory",
                &[1.0, 0.0, 0.0],
            ))
            .unwrap();
        db.put_memory(put("lane-1", "use pglite for comparison", &[0.0, 1.0, 0.0]))
            .unwrap();

        let results = db
            .search_memory(MemorySearch {
                scope_type: Some("lane".to_string()),
                scope_id: Some("lane-1".to_string()),
                query_embedding: Some(vec![1.0, 0.0, 0.0]),
                embedding_provider: Some("test".to_string()),
                embedding_model: Some("tiny".to_string()),
                top_k: 2,
                ..MemorySearch::default()
            })
            .unwrap();
        assert_eq!(results[0].item.memory_id, first.memory_id);
        assert!(results[0].distance.unwrap() <= results[1].distance.unwrap());

        let exact_results = db
            .search_memory(MemorySearch {
                scope_type: Some("lane".to_string()),
                scope_id: Some("lane-1".to_string()),
                query_embedding: Some(vec![1.0, 0.0, 0.0]),
                embedding_provider: Some("test".to_string()),
                embedding_model: Some("tiny".to_string()),
                top_k: 2,
                backend: MemorySearchBackend::Exact,
                ..MemorySearch::default()
            })
            .unwrap();
        assert_eq!(exact_results[0].item.memory_id, first.memory_id);

        let updated = db
            .put_memory(MemoryPut {
                memory_id: Some(first.memory_id.clone()),
                body: "use sqlite vec0 for durable memory".to_string(),
                ..put("lane-1", "ignored", &[1.0, 0.0, 0.0])
            })
            .unwrap();
        assert_eq!(updated.body, "use sqlite vec0 for durable memory");

        let revisions = db.memory_revisions(&first.memory_id).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].version, 1);
        assert_eq!(revisions[1].version, 2);
    }

    #[test]
    fn memory_archive_removes_item_from_active_search() {
        let (_temp, db) = empty_db();
        let item = db
            .put_memory(put("lane-2", "temporary observation", &[0.0, 0.0, 1.0]))
            .unwrap();
        db.archive_memory(
            &item.memory_id,
            "agent:test",
            MemoryVersionSource::default(),
        )
        .unwrap();

        let active = db
            .search_memory(MemorySearch {
                scope_id: Some("lane-2".to_string()),
                query_embedding: Some(vec![0.0, 0.0, 1.0]),
                embedding_provider: Some("test".to_string()),
                embedding_model: Some("tiny".to_string()),
                ..MemorySearch::default()
            })
            .unwrap();
        assert!(active.is_empty());

        let archived = db
            .search_memory(MemorySearch {
                scope_id: Some("lane-2".to_string()),
                status: Some("archived".to_string()),
                ..MemorySearch::default()
            })
            .unwrap();
        assert_eq!(archived.len(), 1);
    }
}
