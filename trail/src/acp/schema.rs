use std::collections::BTreeSet;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

pub(crate) const ACP_V1_SCHEMA_SHA256: &str =
    "92c1dfcda10dd47e99127500a3763da2b471f9ac61e12b9bf0430c32cf953796";
pub(crate) const ACP_V1_META_SHA256: &str =
    "e0bf36f8123b2544b499174197fdc371ec49a1b4572a35114513d56492741599";
pub(crate) const ACP_V1_SCHEMA_COMMIT: &str = "64cbd71ae520b89aac54164d8c1d364333c8ee5f";

const ACP_V1_SCHEMA_BYTES: &[u8] = include_bytes!("../../tests/fixtures/acp/v1/schema.json");
const ACP_V1_META_BYTES: &[u8] = include_bytes!("../../tests/fixtures/acp/v1/meta.json");
const ACP_V1_SOURCE_BYTES: &[u8] = include_bytes!("../../tests/fixtures/acp/v1/source.json");

#[allow(dead_code)]
pub(crate) struct AcpV1Contract {
    schema_sha256: String,
    meta_sha256: String,
    method_names: BTreeSet<String>,
    validator: jsonschema::Validator,
}

#[allow(dead_code)]
impl AcpV1Contract {
    pub(crate) fn load() -> Result<Self> {
        let schema_sha256 = sha256_hex(ACP_V1_SCHEMA_BYTES);
        let meta_sha256 = sha256_hex(ACP_V1_META_BYTES);
        ensure_digest("schema.json", &schema_sha256, ACP_V1_SCHEMA_SHA256)?;
        ensure_digest("meta.json", &meta_sha256, ACP_V1_META_SHA256)?;

        let source: Value = serde_json::from_slice(ACP_V1_SOURCE_BYTES)?;
        ensure_source_manifest(&source)?;

        let schema: Value = serde_json::from_slice(ACP_V1_SCHEMA_BYTES)?;
        let meta: Value = serde_json::from_slice(ACP_V1_META_BYTES)?;
        if meta.get("version").and_then(Value::as_u64) != Some(1) {
            return Err(Error::Corrupt(
                "vendored ACP metadata does not declare wire version 1".to_string(),
            ));
        }

        let mut method_names = BTreeSet::new();
        for group in ["agentMethods", "clientMethods", "protocolMethods"] {
            let methods = meta.get(group).and_then(Value::as_object).ok_or_else(|| {
                Error::Corrupt(format!("vendored ACP metadata is missing `{group}`"))
            })?;
            for method in methods.values() {
                let method = method.as_str().ok_or_else(|| {
                    Error::Corrupt(format!(
                        "vendored ACP metadata `{group}` contains a non-string method"
                    ))
                })?;
                if !method_names.insert(method.to_string()) {
                    return Err(Error::Corrupt(format!(
                        "vendored ACP metadata repeats method `{method}`"
                    )));
                }
            }
        }
        if method_names.len() != 23 {
            return Err(Error::Corrupt(format!(
                "vendored ACP metadata contains {} methods instead of 23",
                method_names.len()
            )));
        }

        let validator = jsonschema::validator_for(&schema).map_err(|err| {
            Error::Corrupt(format!("vendored ACP v1 schema does not compile: {err}"))
        })?;
        Ok(Self {
            schema_sha256,
            meta_sha256,
            method_names,
            validator,
        })
    }

    pub(crate) fn wire_version(&self) -> u16 {
        1
    }

    pub(crate) fn schema_sha256(&self) -> &str {
        &self.schema_sha256
    }

    pub(crate) fn meta_sha256(&self) -> &str {
        &self.meta_sha256
    }

    pub(crate) fn method_names(&self) -> &BTreeSet<String> {
        &self.method_names
    }

    pub(crate) fn validator(&self) -> &jsonschema::Validator {
        &self.validator
    }

    pub(crate) fn validate(&self, message: &Value) -> Result<()> {
        self.validator
            .validate(message)
            .map_err(|err| Error::InvalidInput(format!("message is not valid ACP v1: {err}")))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn ensure_digest(name: &str, actual: &str, expected: &str) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::Corrupt(format!(
            "vendored ACP v1 `{name}` digest is `{actual}`, expected `{expected}`"
        )))
    }
}

fn ensure_source_manifest(source: &Value) -> Result<()> {
    let expected = [
        ("commit", ACP_V1_SCHEMA_COMMIT),
        ("schemaSha256", ACP_V1_SCHEMA_SHA256),
        ("metaSha256", ACP_V1_META_SHA256),
    ];
    for (field, expected) in expected {
        if source.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(Error::Corrupt(format!(
                "vendored ACP v1 source manifest has the wrong `{field}`"
            )));
        }
    }
    if source.get("wireVersion").and_then(Value::as_u64) != Some(1) {
        return Err(Error::Corrupt(
            "vendored ACP v1 source manifest has the wrong `wireVersion`".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_v1_artifacts_match_manifest_and_compile() {
        let contract = AcpV1Contract::load().unwrap();
        assert_eq!(contract.wire_version(), 1);
        assert_eq!(contract.method_names().len(), 23);
        assert_eq!(contract.schema_sha256(), ACP_V1_SCHEMA_SHA256);
        assert_eq!(contract.meta_sha256(), ACP_V1_META_SHA256);
        assert!(contract.validator().is_valid(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": 1, "clientCapabilities": {}}
        })));
    }
}
