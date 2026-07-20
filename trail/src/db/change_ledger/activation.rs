use serde::Serialize;
use sha2::{Digest, Sha256};

const APPROVED_PRODUCER_INVENTORY_SHA256: &str =
    "a13fa0330d89ad442a4f796a5fd37b55177ab4fdf7805354925b99fc18199d0e";
const APPROVED_RAW_MUTATION_INVENTORY_SHA256: &str =
    "21f2de654b01a47669160b991f5c4fcaf2f0613c8a2a1c5b2aff3be971997698";
const APPROVED_ACTIVATION_AUDIT_SHA256: &str =
    "9c4e60f1741a981c1553ac5ed28cfea76ffc58cbd2a712beccbbc1e7ec3a269b";
const ACTIVATION_AUDIT_MANIFEST: &str = concat!(
    "trail-changed-path-activation-v1\n",
    "schema=20\n",
    "producer=a13fa0330d89ad442a4f796a5fd37b55177ab4fdf7805354925b99fc18199d0e\n",
    "raw=21f2de654b01a47669160b991f5c4fcaf2f0613c8a2a1c5b2aff3be971997698\n",
    "linux_suite=changed_path_ledger_linux\n",
    "macos_suite=changed_path_ledger_macos\n",
    "recovery_suite=changed_path_ledger_recovery\n",
    "activation_suite=changed_path_ledger_activation\n",
    "scale_workflow=CLI Scale Benchmark/changed-path-ledger\n",
    "native_workflow=Changed-path Ledger Native Gates/native\n",
    "exact_sha_tag_gate=Release Automation/exact-sha-native-ledger\n",
    "exact_sha_publish_gate=Release/custom-changed-path-ledger-native\n",
    "gate_schema=changed-path-thresholds-v1\n",
    "metrics_schema=operation-metrics-jsonl-v1\n",
);

// These bytes are compiled into the checked binary. Activation never trusts
// a mutable report, CI artifact, environment variable, or file discovered at
// runtime to decide whether command authority is enabled.
const PRODUCER_INVENTORY: &[u8] =
    include_bytes!("../../../tests/fixtures/changed_path_producers.v1");
const RAW_MUTATION_INVENTORY: &[u8] =
    include_bytes!("../../../tests/fixtures/changed_path_raw_mutations.v1");

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ActivationEvidence {
    pub(crate) schema_hard_cutover: bool,
    pub(crate) producer_inventory_complete: bool,
    pub(crate) linux_native_suite: bool,
    pub(crate) macos_native_suite: bool,
    pub(crate) crash_matrix: bool,
    pub(crate) corruption_matrix: bool,
    pub(crate) scale_gates: bool,
    pub(crate) metrics_jsonl: bool,
    pub(crate) exact_sha_tag_gate: bool,
    pub(crate) exact_sha_publish_gate: bool,
    pub(crate) producer_inventory_sha256: String,
    pub(crate) raw_mutation_inventory_sha256: String,
    pub(crate) activation_audit_sha256: String,
}

impl ActivationEvidence {
    pub(crate) fn from_checked_build() -> std::result::Result<Self, String> {
        let producer_inventory_sha256 = sha256(PRODUCER_INVENTORY);
        let raw_mutation_inventory_sha256 = sha256(RAW_MUTATION_INVENTORY);
        let activation_audit_sha256 = sha256(ACTIVATION_AUDIT_MANIFEST.as_bytes());
        if producer_inventory_sha256 != APPROVED_PRODUCER_INVENTORY_SHA256 {
            return Err(format!(
                "controlled producer inventory changed: expected {APPROVED_PRODUCER_INVENTORY_SHA256}, compiled {producer_inventory_sha256}"
            ));
        }
        if raw_mutation_inventory_sha256 != APPROVED_RAW_MUTATION_INVENTORY_SHA256 {
            return Err(format!(
                "raw mutation inventory changed: expected {APPROVED_RAW_MUTATION_INVENTORY_SHA256}, compiled {raw_mutation_inventory_sha256}"
            ));
        }
        if activation_audit_sha256 != APPROVED_ACTIVATION_AUDIT_SHA256 {
            return Err(format!(
                "compiled activation audit changed: expected {APPROVED_ACTIVATION_AUDIT_SHA256}, compiled {activation_audit_sha256}"
            ));
        }
        let checked_activation_audit = activation_audit_sha256 == APPROVED_ACTIVATION_AUDIT_SHA256;
        Ok(Self {
            schema_hard_cutover: checked_activation_audit
                && super::super::TRAIL_SCHEMA_VERSION == 20,
            producer_inventory_complete: checked_activation_audit,
            // These fields declare the checked build contract. Exact-SHA
            // workflow dependencies, rather than this self-hash, authorize a
            // release tag and every cargo-dist publication phase.
            linux_native_suite: checked_activation_audit,
            macos_native_suite: checked_activation_audit,
            crash_matrix: checked_activation_audit,
            corruption_matrix: checked_activation_audit,
            scale_gates: checked_activation_audit,
            metrics_jsonl: checked_activation_audit,
            exact_sha_tag_gate: checked_activation_audit,
            exact_sha_publish_gate: checked_activation_audit,
            producer_inventory_sha256,
            raw_mutation_inventory_sha256,
            activation_audit_sha256,
        })
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.schema_hard_cutover
            && self.producer_inventory_complete
            && self.linux_native_suite
            && self.macos_native_suite
            && self.crash_matrix
            && self.corruption_matrix
            && self.scale_gates
            && self.metrics_jsonl
            && self.exact_sha_tag_gate
            && self.exact_sha_publish_gate
    }
}

pub(crate) fn ledger_authority_enabled_for(platform: &str, evidence: &ActivationEvidence) -> bool {
    matches!(platform, "linux" | "macos") && evidence.is_complete()
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_platform_and_incomplete_evidence_are_hard_off() {
        let complete = ActivationEvidence::from_checked_build().unwrap();
        assert!(!ledger_authority_enabled_for("windows", &complete));
        assert!(!ledger_authority_enabled_for("freebsd", &complete));
        let mut incomplete = complete;
        incomplete.corruption_matrix = false;
        assert!(!ledger_authority_enabled_for("linux", &incomplete));
        assert!(!ledger_authority_enabled_for("macos", &incomplete));
    }
}
