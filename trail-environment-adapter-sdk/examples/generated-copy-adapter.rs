use trail_environment_adapter_sdk::{
    serve_once, AdapterCommand, AdapterOperation, AdapterOutput, AdapterPlan, AdapterResponse,
    AdapterResult, DiscoveredComponent, PROTOCOL_V1,
};

fn main() {
    if let Err(error) = serve_once(|request| {
        if request.protocol != PROTOCOL_V1 || request.adapter_identity != "example/copy@1" {
            return AdapterResponse::for_request(
                &request,
                AdapterResult::Error {
                    code: "unsupported_request".to_string(),
                    message: "protocol or adapter identity does not match".to_string(),
                },
            );
        }
        let result = match &request.operation {
            AdapterOperation::Discover { files, .. } => AdapterResult::Discovered {
                component: files
                    .iter()
                    .any(|file| file.path == "copy.adapter")
                    .then(|| DiscoveredComponent::new("plugin.copy", "generated")),
            },
            AdapterOperation::Plan {
                component_id,
                files,
                ..
            } => {
                let writable_private = files.iter().any(|file| {
                    file.path == "copy.adapter" && file.content.starts_with(b"writable_private")
                });
                let (program, args) = if request.host.operating_system == "windows" {
                    (
                        "tar.exe".to_string(),
                        vec![
                            "-cf".to_string(),
                            "generated/out.tar".to_string(),
                            "input.txt".to_string(),
                        ],
                    )
                } else {
                    (
                        "cp".to_string(),
                        vec!["input.txt".to_string(), "generated/copied.txt".to_string()],
                    )
                };
                let output = if writable_private {
                    AdapterOutput::writable_private(
                        "generated",
                        "generated",
                        ".trail-generated/plugin-copy",
                    )
                } else {
                    AdapterOutput::immutable_seed_private(
                        "generated",
                        "generated",
                        ".trail-generated/plugin-copy",
                    )
                };
                match AdapterPlan::builder(component_id.clone(), "generated")
                    .identity_inputs(files.iter().map(|file| file.path.clone()))
                    .semantic_input(
                        "strategy",
                        if writable_private {
                            "copy-writable-private-v1"
                        } else {
                            "copy-immutable-seed-v1"
                        },
                    )
                    .command(AdapterCommand::new(program, args))
                    .output(output)
                    .stale_reason("pinned plugin inputs, executable, platform, or adapter changed")
                    .build()
                {
                    Ok(plan) => AdapterResult::Planned { plan },
                    Err(error) => AdapterResult::Error {
                        code: "invalid_plan".to_string(),
                        message: error.to_string(),
                    },
                }
            }
        };
        AdapterResponse::for_request(&request, result)
    }) {
        eprintln!("generated-copy-adapter: {error}");
        std::process::exit(1);
    }
}
