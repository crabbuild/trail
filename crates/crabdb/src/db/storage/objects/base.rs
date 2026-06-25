use super::*;

impl CrabDb {
    pub(crate) fn put_text_content_from_lines(&self, lines: &[LineEntry]) -> Result<ObjectId> {
        let bytes = materialize_lines(lines);
        let mut order_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut index_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        for (idx, entry) in lines.iter().enumerate() {
            let key = order_key(idx as u64 + 1);
            order_builder.add(key.clone(), cbor(entry)?);
            index_builder.add(entry.line_id.encode_key(), key);
        }
        let order_tree = order_builder.build()?;
        let index_tree = index_builder.build()?;
        let content = TextContent {
            version: TEXT_OBJECT_VERSION,
            content_hash: sha256_hex(&bytes),
            line_count: lines.len() as u64,
            byte_count: bytes.len() as u64,
            order_map_root: tree_root_hex(&order_tree),
            line_index_map_root: tree_root_hex(&index_tree),
            representation: TextRepresentation::TreeText,
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
                bytes,
                now_ts()
            ],
        )?;
        Ok(object_id)
    }

    pub(crate) fn get_object<T: serde::de::DeserializeOwned>(
        &self,
        kind: &'static str,
        object_id: &ObjectId,
    ) -> Result<T> {
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
        from_cbor(&bytes)
    }
}
