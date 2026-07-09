use super::*;

impl Trail {
    pub(crate) fn put_text_content_from_lines(&self, lines: &[LineEntry]) -> Result<ObjectId> {
        let bytes = materialize_lines(lines);
        let content_hash = sha256_hex(&bytes);
        let small_text = self.config.text.small_text_max_bytes > 0
            && bytes.len() as u64 <= self.config.text.small_text_max_bytes;
        let (order_map_root, line_index_map_root, representation) = if small_text {
            (
                None,
                None,
                TextRepresentation::SmallTextTable {
                    table: encode_small_text_table(lines),
                },
            )
        } else {
            let mut order_builder = BatchBuilder::new(self.store.clone(), prolly_config());
            let mut index_builder = BatchBuilder::new(self.store.clone(), prolly_config());
            for (idx, entry) in lines.iter().enumerate() {
                let key = order_key(idx as u64 + 1);
                order_builder.add(key.clone(), cbor(entry)?);
                index_builder.add(entry.line_id.encode_key(), key);
            }
            let order_tree = order_builder.build()?;
            let index_tree = index_builder.build()?;
            (
                tree_root_hex(&order_tree),
                tree_root_hex(&index_tree),
                TextRepresentation::TreeText,
            )
        };
        let full_bytes_blob_id = if small_text {
            None
        } else {
            Some(self.put_blob(bytes.clone())?)
        };
        let content = TextContent {
            version: TEXT_OBJECT_VERSION,
            content_hash,
            line_count: lines.len() as u64,
            byte_count: bytes.len() as u64,
            full_bytes_blob_id,
            order_map_root,
            line_index_map_root,
            representation,
        };
        self.put_object(TEXT_CONTENT_KIND, TEXT_OBJECT_VERSION, &content)
    }

    pub(crate) fn put_blob(&self, bytes: Vec<u8>) -> Result<ObjectId> {
        let blob = Blob {
            version: BLOB_OBJECT_VERSION,
            content_hash: sha256_hex(&bytes),
            bytes,
        };
        self.put_object(BLOB_KIND, BLOB_OBJECT_VERSION, &blob)
    }

    pub(crate) fn put_object<T: Serialize>(
        &self,
        kind: &str,
        version: u16,
        value: &T,
    ) -> Result<ObjectId> {
        let bytes = cbor(value)?;
        let object_id = ObjectId::for_bytes(kind, version, &bytes);
        self.conn.execute(
            "INSERT OR IGNORE INTO objects \
             (object_id, kind, version, codec, hash_alg, size_bytes, bytes, created_at) \
             VALUES (?1, ?2, ?3, 'cbor', 'sha256', ?4, ?5, ?6)",
            params![
                object_id.0,
                kind,
                version as i64,
                bytes.len() as i64,
                bytes.as_slice(),
                now_ts()
            ],
        )?;
        self.cache_object_bytes(&object_id, kind, &bytes);
        Ok(object_id)
    }

    pub(crate) fn get_object<T: serde::de::DeserializeOwned>(
        &self,
        kind: &'static str,
        object_id: &ObjectId,
    ) -> Result<T> {
        if let Some(bytes) = self.cached_object_bytes(kind, object_id) {
            return from_cbor(&bytes);
        }
        let row: Option<(String, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT kind, bytes FROM objects WHERE object_id = ?1",
                params![object_id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((actual_kind, bytes)) = row else {
            return Err(Error::ObjectNotFound {
                kind,
                id: object_id.0.clone(),
            });
        };
        if actual_kind != kind {
            return Err(Error::Corrupt(format!(
                "object {} has kind {}, expected {}",
                object_id.0, actual_kind, kind
            )));
        }
        self.cache_object_bytes(object_id, kind, &bytes);
        from_cbor(&bytes)
    }

    pub(crate) fn get_objects<T: serde::de::DeserializeOwned>(
        &self,
        kind: &'static str,
        object_ids: &[ObjectId],
    ) -> Result<HashMap<ObjectId, T>> {
        let wanted = object_ids.iter().cloned().collect::<BTreeSet<_>>();
        let wanted = wanted.into_iter().collect::<Vec<_>>();
        let mut out = HashMap::new();
        let mut missing = Vec::new();
        for object_id in &wanted {
            if let Some(bytes) = self.cached_object_bytes(kind, object_id) {
                out.insert(object_id.clone(), from_cbor(&bytes)?);
            } else {
                missing.push(object_id.clone());
            }
        }
        for chunk in missing.chunks(512) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT object_id, kind, bytes FROM objects WHERE object_id IN ({placeholders})"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(
                params_from_iter(chunk.iter().map(|id| id.0.as_str())),
                |row| {
                    Ok((
                        ObjectId(row.get::<_, String>(0)?),
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )?;
            for row in rows {
                let (object_id, actual_kind, bytes) = row?;
                if actual_kind != kind {
                    return Err(Error::Corrupt(format!(
                        "object {} has kind {}, expected {}",
                        object_id.0, actual_kind, kind
                    )));
                }
                self.cache_object_bytes(&object_id, kind, &bytes);
                out.insert(object_id, from_cbor(&bytes)?);
            }
        }
        for object_id in wanted {
            if !out.contains_key(&object_id) {
                return Err(Error::ObjectNotFound {
                    kind,
                    id: object_id.0,
                });
            }
        }
        Ok(out)
    }

    fn cached_object_bytes(&self, kind: &'static str, object_id: &ObjectId) -> Option<Vec<u8>> {
        self.object_cache
            .lock()
            .expect("object cache poisoned")
            .get(kind, object_id)
    }

    fn cache_object_bytes(&self, object_id: &ObjectId, kind: &str, bytes: &[u8]) {
        self.object_cache
            .lock()
            .expect("object cache poisoned")
            .insert(object_id, kind, bytes);
    }
}
