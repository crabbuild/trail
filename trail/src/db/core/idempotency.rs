use super::*;

impl Trail {
    pub(crate) fn http_idempotency_entry(&self, key: &str) -> Result<Option<HttpIdempotencyEntry>> {
        self.conn
            .query_row(
                "SELECT method, path, request_hash, status, body \
                 FROM http_idempotency_keys WHERE key = ?1",
                params![key],
                |row| {
                    let status = row.get::<_, i64>(3)?;
                    Ok(HttpIdempotencyEntry {
                        method: row.get(0)?,
                        path: row.get(1)?,
                        request_hash: row.get(2)?,
                        status: status as u16,
                        body: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn store_http_idempotency_response(
        &mut self,
        input: HttpIdempotencyStoreInput,
    ) -> Result<()> {
        let _lock = self.acquire_write_lock()?;
        let now = now_ts();
        self.conn.execute(
            "INSERT OR IGNORE INTO http_idempotency_keys \
             (key, method, path, request_hash, status, body, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                input.key,
                input.method,
                input.path,
                input.request_hash,
                input.status as i64,
                input.body,
                now
            ],
        )?;
        Ok(())
    }
}
