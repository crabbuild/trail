use std::collections::BTreeMap;

use trail_environment_adapter_sdk::{
    denied_capabilities_v3, serve_once_v3, AdapterActionLimitsV3, AdapterActionPhaseV3,
    AdapterActionV3, AdapterComponentProposalV3, AdapterIdentityContractV3, AdapterInputRoleV3,
    AdapterInputV3, AdapterOperationV3, AdapterOutput, AdapterPipelineV3, AdapterPortability,
    AdapterProcessCapabilityV3, AdapterProposalStatusV3, AdapterQuarantinePolicyV3,
    AdapterResponseV3, AdapterResultV3, AdapterSourceExportAuthorizationV3,
    AdapterSourceExportCollisionV3, AdapterSourceExportV3, AdapterValidationKindV3,
    AdapterValidationV3, PROTOCOL_V3,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    serve_once_v3(|request| {
        let result = if request.protocol != PROTOCOL_V3
            || request.adapter_identity != "example/artifact-pipeline@1"
        {
            AdapterResultV3::Error {
                code: "unsupported_request".into(),
                message: "expected the example protocol-v3 adapter identity".into(),
                recovery_actions: Vec::new(),
            }
        } else {
            match &request.operation {
                AdapterOperationV3::Propose {
                    component_root,
                    files,
                } => AdapterResultV3::Proposed {
                    component: files
                        .iter()
                        .any(|file| file.path == "schema.json")
                        .then(|| AdapterComponentProposalV3 {
                            component_id: "example.generated-client".into(),
                            component_root: component_root.clone(),
                            kind: "generated".into(),
                            status: AdapterProposalStatusV3::Ready,
                            proposal_key: format!("{}:schema", request.source_root),
                            missing_requirements: Vec::new(),
                            recovery_actions: Vec::new(),
                        }),
                },
                AdapterOperationV3::Plan {
                    proposal, files, ..
                } => {
                    let mut builder = AdapterPipelineV3::builder(
                        (**proposal).clone(),
                        AdapterIdentityContractV3 {
                            normalizer_version: "trail-path-v1".into(),
                            source_closure_complete: true,
                            semantic_identities: BTreeMap::from([(
                                "generator_mode".into(),
                                "client".into(),
                            )]),
                            target: "host".into(),
                            platform: request.host.operating_system.clone(),
                            architecture: request.host.architecture.clone(),
                            abi: "host".into(),
                            portability: AdapterPortability::Host,
                            portability_certified: false,
                            portability_scope: "workspace".into(),
                            trust_scope: "local_plugin".into(),
                        },
                    );
                    for pinned in files {
                        builder = builder.input(AdapterInputV3 {
                            role: AdapterInputRoleV3::Identity,
                            required: true,
                            ..pinned.input.clone()
                        });
                    }
                    let pipeline = builder
                        .action(AdapterActionV3 {
                            name: "construct".into(),
                            phase: AdapterActionPhaseV3::Construct,
                            program: "example-codegen".into(),
                            argv: vec!["build".into(), "--output".into(), "generated".into()],
                            working_directory: ".".into(),
                            environment: BTreeMap::new(),
                            capabilities:
                                trail_environment_adapter_sdk::AdapterCapabilityProfileV3 {
                                    process: AdapterProcessCapabilityV3::DeclaredExecutable,
                                    ..denied_capabilities_v3()
                                },
                            limits: AdapterActionLimitsV3 {
                                timeout_ms: 30_000,
                                stdout_bytes: 1024 * 1024,
                                stderr_bytes: 1024 * 1024,
                                output_entries: 10_000,
                                output_bytes: 64 * 1024 * 1024,
                                child_processes: 0,
                            },
                        })
                        .validation(AdapterValidationV3 {
                            name: "path-contract".into(),
                            kind: AdapterValidationKindV3::PathContract,
                            path: "generated".into(),
                            required: true,
                            parameters: BTreeMap::new(),
                        })
                        .output(AdapterOutput::immutable_seed_private(
                            "generated",
                            "generated",
                            ".trail-generated/generated",
                        ))
                        .source_export(AdapterSourceExportV3 {
                            name: "client".into(),
                            output_name: "generated".into(),
                            artifact_subpath: "client".into(),
                            destination: "src/generated".into(),
                            collision: AdapterSourceExportCollisionV3::Fail,
                            required_validation: "path-contract".into(),
                            required_gate: None,
                            authorization: AdapterSourceExportAuthorizationV3::ExplicitUser,
                        })
                        .quarantine_policy(AdapterQuarantinePolicyV3::FailClosed)
                        .stale_reason("schema, generator, or host identity changed")
                        .build();
                    match pipeline {
                        Ok(pipeline) => AdapterResultV3::Planned {
                            pipeline: Box::new(pipeline),
                        },
                        Err(error) => AdapterResultV3::Error {
                            code: "invalid_plan".into(),
                            message: error.to_string(),
                            recovery_actions: Vec::new(),
                        },
                    }
                }
            }
        };
        AdapterResponseV3::for_request(&request, result)
    })?;
    Ok(())
}
