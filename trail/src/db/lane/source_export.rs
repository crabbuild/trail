use super::workspace_artifact::ArtifactLazyEntry;
use super::*;
use crate::ids::{ArtifactEnvelopeId, ArtifactTreeId};

const ARTIFACT_SOURCE_EXPORT_PLAN_VERSION: u16 = 1;

impl Trail {
    /// Plan one explicit generated-source export without writing source.
    ///
    /// The returned evidence pins every authority consumed by execution. A
    /// later writer must reject the plan if any source, generation, artifact,
    /// validation, gate, or destination pin changes.
    pub fn plan_artifact_source_export(
        &self,
        lane: &str,
        component_id: &str,
        export_name: &str,
        authorization: ArtifactSourceExportAuthorizationV1,
    ) -> Result<ArtifactSourceExportPlanV1> {
        let lane_details = self.lane_details(lane)?;
        let branch = &lane_details.branch;
        let generation = self
            .active_environment_generation(&lane_details.record.name)?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "lane `{}` has no active environment generation to export",
                    lane_details.record.name
                ))
            })?;
        if generation.state != "active" {
            return Err(Error::InvalidInput(format!(
                "environment generation `{}` is not active",
                generation.generation_id
            )));
        }
        let contract = self
            .command_recipe_source_exports(&generation.source_root, component_id)?
            .into_iter()
            .find(|contract| contract.name == export_name)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "repository component `{component_id}` has no source export named `{export_name}`"
                ))
            })?;
        if contract.authorization_mode != "explicit"
            || authorization != ArtifactSourceExportAuthorizationV1::ExplicitUser
        {
            return Err(Error::InvalidInput(format!(
                "source export `{export_name}` requires explicit user authorization"
            )));
        }
        let collision_mode = match contract.collision_policy.as_str() {
            "fail" => ArtifactSourceExportCollisionModeV1::Fail,
            "replace" => ArtifactSourceExportCollisionModeV1::Replace,
            other => {
                return Err(Error::Corrupt(format!(
                    "source export `{export_name}` retained unsupported collision mode `{other}`"
                )))
            }
        };

        let binding = self
            .conn
            .query_row(
                "SELECT b.desired_key,b.envelope_id,b.tree_root_id,o.layer_subpath
                 FROM artifact_generation_bindings b
                 JOIN environment_generation_outputs o
                   ON o.generation_id=b.generation_id
                  AND o.component_id=b.component_id
                  AND o.output_name=b.output_name
                 WHERE b.generation_id=?1 AND b.component_id=?2 AND b.output_name=?3",
                params![
                    &generation.generation_id,
                    component_id,
                    &contract.output_name
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "active generation `{}` has no sealed artifact binding for `{component_id}/{}`",
                    generation.generation_id, contract.output_name
                ))
            })?;
        let (binding_desired_key, envelope_id, tree_root_id, layer_subpath) = binding;
        let envelope_id = ArtifactEnvelopeId::parse(envelope_id).map_err(|error| {
            Error::Corrupt(format!("invalid source-export envelope ID: {error}"))
        })?;
        let tree_root_id = ArtifactTreeId::parse(tree_root_id).map_err(|error| {
            Error::Corrupt(format!("invalid source-export tree-root ID: {error}"))
        })?;
        let envelope =
            self.verify_ready_artifact_envelope_under_write_lock(&envelope_id, &tree_root_id)?;
        if !envelope.secret_taint.is_clear() {
            return Err(Error::InvalidInput(format!(
                "source export `{export_name}` cannot use secret-tainted artifact output"
            )));
        }
        let encoded_desired_key = match &envelope.desired_identity {
            ArtifactDesiredIdentityV1::WorkspaceLayerV1 { cache_key, .. } => cache_key,
            ArtifactDesiredIdentityV1::ArtifactDesiredV2 { desired_key } => &desired_key.0,
        };
        if encoded_desired_key != &binding_desired_key {
            return Err(Error::Corrupt(format!(
                "source-export artifact binding for `{component_id}/{}` disagrees with envelope desired identity",
                contract.output_name
            )));
        }

        let artifact_subpath = normalize_relative_path(&join_source_export_path(
            &layer_subpath,
            &contract.artifact_subpath,
        ))?;
        let subtree = match self.artifact_tree_lazy_entry(&tree_root_id, &artifact_subpath)? {
            Some(ArtifactLazyEntry::Directory { node_id }) => {
                ArtifactSourceExportSubtreeV1::Directory { node_id }
            }
            Some(ArtifactLazyEntry::File { node_id, .. }) => {
                ArtifactSourceExportSubtreeV1::File { node_id }
            }
            Some(ArtifactLazyEntry::Symlink { .. }) => {
                return Err(Error::InvalidInput(format!(
                    "source export `{export_name}` cannot select a symlink as its artifact root"
                )))
            }
            None => {
                return Err(Error::InvalidInput(format!(
                    "source export `{export_name}` artifact subtree `{}` does not exist",
                    contract.artifact_subpath
                )))
            }
        };

        let mut matching_validation_receipts = Vec::new();
        for receipt_id in &envelope.validation_receipt_ids {
            let receipt = self.artifact_validation_receipt(receipt_id)?;
            if receipt.declaration.name == contract.required_validation
                && receipt.outcome == ArtifactValidationOutcomeV1::Passed
                && receipt.desired_identity == envelope.desired_identity
                && receipt.tree_root_id == tree_root_id
            {
                matching_validation_receipts.push(receipt_id.clone());
            }
        }
        if matching_validation_receipts.len() != 1 {
            return Err(Error::InvalidInput(format!(
                "source export `{export_name}` requires exactly one passed validation `{}` for the exact artifact",
                contract.required_validation
            )));
        }
        let validation_receipt_id = matching_validation_receipts.remove(0);

        let gate = contract
            .required_gate
            .as_deref()
            .map(|kind| {
                let gate = self
                    .latest_lane_gate(&branch.lane_id, kind)?
                    .ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "source export `{export_name}` requires a recorded `{kind}` gate"
                        ))
                    })?;
                if !gate.success
                    || gate.source_root.as_ref() != Some(&branch.head_root)
                    || gate.view_id.as_deref() != Some(&generation.view_id)
                {
                    return Err(Error::InvalidInput(format!(
                        "source export `{export_name}` requires a successful `{kind}` gate pinned to the current source and environment view"
                    )));
                }
                Ok(ArtifactSourceExportGatePinV1 {
                    event_id: gate.event_id,
                    kind: kind.to_string(),
                    source_root: branch.head_root.clone(),
                    view_id: generation.view_id.clone(),
                    view_generation: gate.view_generation.ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "source export `{export_name}` gate `{kind}` lacks a view-generation pin"
                        ))
                    })?,
                })
            })
            .transpose()?;

        let destination = normalize_relative_path(&contract.destination)?;
        let destination_pin =
            self.source_export_destination_pin(&branch.head_root, &destination)?;
        if collision_mode == ArtifactSourceExportCollisionModeV1::Fail && destination_pin.exists {
            return Err(Error::Conflict(format!(
                "source export `{export_name}` destination `{destination}` already exists and collision mode is `fail`"
            )));
        }

        Ok(ArtifactSourceExportPlanV1 {
            version: ARTIFACT_SOURCE_EXPORT_PLAN_VERSION,
            lane_id: branch.lane_id.clone(),
            lane: lane_details.record.name,
            component_id: component_id.to_string(),
            export_name: export_name.to_string(),
            output_name: contract.output_name,
            source_root: branch.head_root.clone(),
            generation_id: generation.generation_id,
            generation_source_root: generation.source_root,
            desired_identity: envelope.desired_identity,
            envelope_id,
            tree_root_id,
            subtree,
            artifact_subpath,
            destination,
            destination_pin,
            collision_mode,
            validation_receipt_id,
            gate,
            authorization,
        })
    }

    fn source_export_destination_pin(
        &self,
        source_root: &ObjectId,
        destination: &str,
    ) -> Result<ArtifactSourceExportDestinationPinV1> {
        let selected =
            self.load_root_files_for_selections(source_root, &[destination.to_string()])?;
        let logical_bytes = selected.values().try_fold(0u64, |total, entry| {
            total.checked_add(entry.size_bytes).ok_or_else(|| {
                Error::InvalidInput("source-export destination byte count overflowed".into())
            })
        })?;
        let exists = !selected.is_empty();
        let content_digest = exists
            .then(|| serde_json::to_vec(&selected).map(|bytes| sha256_hex(&bytes)))
            .transpose()?;
        Ok(ArtifactSourceExportDestinationPinV1 {
            exists,
            content_digest,
            entry_count: selected.len() as u64,
            logical_bytes,
        })
    }
}

fn join_source_export_path(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_string()
    } else {
        format!("{parent}/{child}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_source_export_fixture(
        collision: &str,
        gate: Option<&str>,
        existing_destination: bool,
    ) -> (tempfile::TempDir, Trail) {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("input.txt"), "identity\n").unwrap();
        if existing_destination {
            fs::create_dir_all(workspace.path().join("src/generated-client")).unwrap();
            fs::write(
                workspace.path().join("src/generated-client/old.rs"),
                "old\n",
            )
            .unwrap();
        }
        let gate = gate
            .map(|gate| format!("gate = {gate:?}\n"))
            .unwrap_or_default();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            format!(
                r#"schema = "trail.environment/v2"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "custom.export"
adapter = "trail/command@1"
inputs = [{{ path = "input.txt" }}]

[component.build]
command = ["cp", "input.txt", "generated/generated-client/new.rs"]

[[component.output]]
name = "generated"
source = "generated"
target = ".trail-generated/export"
policy = "immutable_seed_private"
reuse = "exact"
scope = "workspace"
publish = "on_sync"

[[component.source_export]]
name = "client"
from_output = "generated"
source = "generated-client"
target = "src/generated-client"
mode = "explicit"
collision = "{collision}"
{gate}"#
            ),
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "export",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();

        let candidate = tempfile::tempdir().unwrap();
        fs::create_dir_all(candidate.path().join("outputs/0000/generated-client")).unwrap();
        fs::write(
            candidate
                .path()
                .join("outputs/0000/generated-client/new.rs"),
            "generated\n",
        )
        .unwrap();
        let branch = db.lane_branch("export").unwrap();
        let view = db.lane_workspace_view("export").unwrap().unwrap();
        let layer_key = WorkspaceLayerKeyV1 {
            kind: "generated".into(),
            adapter: "command".into(),
            adapter_version: 1,
            inputs: BTreeMap::from([("source_root".into(), branch.head_root.0.clone())]),
            tool_versions: BTreeMap::from([("cp".into(), "fixture".into())]),
            platform: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            portability_scope: "workspace".into(),
            strategy: "source-export-fixture".into(),
        };
        let cache_key = db.workspace_layer_cache_key(&layer_key).unwrap();
        let _lock = db.acquire_write_lock().unwrap();
        let (tree_root_id, _) = db
            .ingest_artifact_tree_under_write_lock(candidate.path())
            .unwrap();
        let envelope_id = db
            .put_legacy_artifact_envelope_under_write_lock(
                &layer_key,
                &cache_key,
                tree_root_id.clone(),
            )
            .unwrap();
        let generation_id = "env_generation_source_export_fixture";
        db.conn
            .execute(
                "INSERT INTO environment_generations(
                     generation_id,view_id,generation_sequence,source_root,specification_digest,
                     predecessor_generation_id,state,created_at,activated_at,retired_at)
                 VALUES(?1,?2,1,?3,'source-export-spec',NULL,'active',1,1,NULL)",
                params![generation_id, &view.view_id, branch.head_root.0],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO environment_generation_components(
                     generation_id,component_id,adapter_identity,kind,component_key,layer_id,mount_path)
                 VALUES(?1,'custom.export','trail/command@1','generated',?2,NULL,NULL)",
                params![generation_id, &cache_key],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO environment_generation_outputs(
                     generation_id,component_id,output_name,policy,reuse_mode,sharing_scope,
                     publication_trigger,storage_identity,layer_id,mount_path,layer_subpath)
                 VALUES(?1,'custom.export','generated','immutable_seed_private','exact','workspace',
                        'on_sync',?2,NULL,'.trail-generated/export','outputs/0000')",
                params![generation_id, &envelope_id.0],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO artifact_generation_bindings(
                     binding_id,generation_id,component_id,output_name,desired_key,envelope_id,
                     tree_root_id,binding_identity,created_at)
                 VALUES('binding_source_export',?1,'custom.export','generated',?2,?3,?4,
                        'source-export-binding',1)",
                params![generation_id, &cache_key, &envelope_id.0, &tree_root_id.0],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO environment_view_generations(view_id,generation_id,updated_at)
                 VALUES(?1,?2,1)",
                params![&view.view_id, generation_id],
            )
            .unwrap();
        drop(_lock);
        (workspace, db)
    }

    #[test]
    fn source_export_plan_pins_artifact_subtree_destination_and_authorization() {
        let (_workspace, db) = setup_source_export_fixture("fail", None, false);
        let plan = db
            .plan_artifact_source_export(
                "export",
                "custom.export",
                "client",
                ArtifactSourceExportAuthorizationV1::ExplicitUser,
            )
            .unwrap();
        assert_eq!(plan.version, ARTIFACT_SOURCE_EXPORT_PLAN_VERSION);
        assert_eq!(plan.export_name, "client");
        assert_eq!(plan.output_name, "generated");
        assert_eq!(plan.artifact_subpath, "outputs/0000/generated-client");
        assert!(matches!(
            plan.subtree,
            ArtifactSourceExportSubtreeV1::Directory { .. }
        ));
        assert_eq!(plan.destination, "src/generated-client");
        assert!(!plan.destination_pin.exists);
        assert_eq!(
            plan.authorization,
            ArtifactSourceExportAuthorizationV1::ExplicitUser
        );
        assert_eq!(
            db.artifact_validation_receipt(&plan.validation_receipt_id)
                .unwrap()
                .declaration
                .name,
            super::super::workspace_artifact::HOST_WORKSPACE_LAYER_STRUCTURAL_SEAL
        );
    }

    #[test]
    fn source_export_plan_rejects_missing_gate_and_fail_collision() {
        let (_workspace, db) = setup_source_export_fixture("replace", Some("test"), false);
        let error = db
            .plan_artifact_source_export(
                "export",
                "custom.export",
                "client",
                ArtifactSourceExportAuthorizationV1::ExplicitUser,
            )
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("requires a recorded `test` gate"));

        let (_workspace, db) = setup_source_export_fixture("fail", None, true);
        let error = db
            .plan_artifact_source_export(
                "export",
                "custom.export",
                "client",
                ArtifactSourceExportAuthorizationV1::ExplicitUser,
            )
            .unwrap_err();
        assert!(error.to_string().contains("collision mode is `fail`"));
    }
}
