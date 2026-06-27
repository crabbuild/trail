use sha2::{Digest, Sha256};

use crate::db::HttpIdempotencyStoreInput;
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{CrabDb, Error, Result};

use super::utils::reason_for_status;

pub(super) struct HttpIdempotency {
    key: String,
    method: String,
    path: String,
    request_hash: String,
}

impl HttpIdempotency {
    pub(super) fn from_request(request: &HttpRequest) -> Result<Option<Self>> {
        if !matches!(request.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
            return Ok(None);
        }
        let Some(raw_key) = request.headers.get("idempotency-key") else {
            return Ok(None);
        };
        let key = raw_key.trim();
        if key.is_empty() {
            return Err(Error::InvalidInput(
                "Idempotency-Key header cannot be empty".to_string(),
            ));
        }
        if key.len() > 200 || key.chars().any(char::is_control) {
            return Err(Error::InvalidInput(
                "Idempotency-Key header must be 1-200 non-control characters".to_string(),
            ));
        }
        Ok(Some(Self {
            key: key.to_string(),
            method: request.method.clone(),
            path: request.path.clone(),
            request_hash: request_hash(request),
        }))
    }

    pub(super) fn replay(&self, db: &CrabDb) -> Result<Option<HttpResponse>> {
        let Some(entry) = db.http_idempotency_entry(&self.key)? else {
            return Ok(None);
        };
        if entry.method != self.method
            || entry.path != self.path
            || entry.request_hash != self.request_hash
        {
            return Err(Error::InvalidInput(format!(
                "Idempotency-Key `{}` was already used for a different request",
                self.key
            )));
        }
        Ok(Some(HttpResponse {
            status: entry.status,
            reason: reason_for_status(entry.status),
            extra_headers: Vec::new(),
            body: entry.body,
        }))
    }

    pub(super) fn store(&self, db: &mut CrabDb, response: &HttpResponse) {
        if matches!(response.status, 401 | 403) {
            return;
        }
        let _ = db.store_http_idempotency_response(HttpIdempotencyStoreInput {
            key: self.key.clone(),
            method: self.method.clone(),
            path: self.path.clone(),
            request_hash: self.request_hash.clone(),
            status: response.status,
            body: response.body.clone(),
        });
    }
}

fn request_hash(request: &HttpRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(request.method.as_bytes());
    hasher.update([0]);
    hasher.update(request.path.as_bytes());
    hasher.update([0]);
    hasher.update(&request.body);
    format!("{:x}", hasher.finalize())
}
