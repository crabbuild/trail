use std::fs;

use trail::{
    ArtifactEnvelopeId, ArtifactQuarantineId, ArtifactQuarantineResolutionV1,
    ArtifactResolutionBatchReportV1, ArtifactResolutionComponentReportV1,
    ArtifactResolutionRequestV1, ArtifactVerificationLevelV1, Error, InitImportMode, Result, Trail,
};

#[test]
fn public_artifact_operations_share_serializable_reports_and_bounded_empty_state() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("README.md"), "artifact public API\n").unwrap();
    Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let db = Trail::open(workspace.path()).unwrap();

    let space = db.workspace_artifact_space().unwrap();
    assert_eq!(space.scope, "workspace");
    assert_eq!(space.envelope_count, 0);
    assert_eq!(space.active_quarantine_count, 0);
    assert_eq!(
        serde_json::to_value(&space).unwrap()["storage"]["logical_bytes"],
        0
    );

    let quarantines = db.artifact_quarantine_list_report().unwrap();
    assert_eq!(quarantines.active_count, 0);
    assert_eq!(quarantines.resolved_count, 0);
    assert!(quarantines.quarantines.is_empty());
    assert_eq!(
        serde_json::to_value(ArtifactVerificationLevelV1::Reproduce).unwrap(),
        "reproduce"
    );

    let missing = ArtifactEnvelopeId::new(b"missing-public-artifact");
    for error in [
        db.inspect_artifact(&missing).unwrap_err(),
        db.artifact_content_reachability(&missing).unwrap_err(),
        db.verify_artifact(&missing, ArtifactVerificationLevelV1::Attach)
            .unwrap_err(),
    ] {
        assert!(matches!(error, Error::ObjectNotFound { .. }));
    }
}

#[test]
fn public_artifact_mutation_signatures_remain_typed() {
    let _resolve_component: fn(
        &Trail,
        ArtifactResolutionRequestV1,
        bool,
    ) -> Result<ArtifactResolutionComponentReportV1> = Trail::resolve_artifact_component;
    let _resolve_all: fn(
        &Trail,
        Vec<ArtifactResolutionRequestV1>,
        bool,
    ) -> Result<ArtifactResolutionBatchReportV1> = Trail::resolve_all_artifact_components;
    let _resolve_quarantine: fn(
        &Trail,
        &ArtifactQuarantineId,
        ArtifactQuarantineResolutionV1,
    ) -> Result<trail::ArtifactQuarantineResolutionReportV1> =
        Trail::resolve_artifact_quarantine_report;
}
