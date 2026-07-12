use trail_environment_adapter_sdk::{
    serve_once, AdapterCommand, AdapterOperation, AdapterOutput, AdapterPlanV2, AdapterResponse,
    AdapterResult, DiscoveredComponent, PROTOCOL_V2,
};

fn main() {
    if let Err(error) = serve_once(|request| {
        if request.protocol != PROTOCOL_V2 || request.adapter_identity != "example/mounted@1" {
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
                    .any(|file| file.path == "mounted.adapter")
                    .then(|| DiscoveredComponent::new("plugin.mounted", "generated")),
            },
            AdapterOperation::Plan {
                component_id,
                files,
                ..
            } => {
                let behavior = files
                    .iter()
                    .find(|file| file.path == "mounted.adapter")
                    .map(|file| String::from_utf8_lossy(&file.content).trim().to_string())
                    .unwrap_or_else(|| "success".to_string());
                let (action, path, input) = match behavior.as_str() {
                    "fail" => ("fail", ".trail-generated/plugin-mounted/partial.txt", None),
                    "hang" => ("hang", ".trail-generated/plugin-mounted/partial.txt", None),
                    "source_write" => ("source_write", "source-leak.txt", None),
                    "source_read" => (
                        "source_read",
                        ".trail-generated/plugin-mounted/leaked.txt",
                        Some("input.txt"),
                    ),
                    _ => (
                        "success",
                        ".trail-generated/plugin-mounted/initialized.txt",
                        Some("mounted.adapter"),
                    ),
                };
                let mut arguments = vec![action, path];
                arguments.extend(input);
                match AdapterPlanV2::builder(component_id.clone(), "generated")
                    .identity_inputs(files.iter().map(|file| file.path.clone()))
                    .semantic_input("behavior", behavior)
                    .mounted_command(AdapterCommand::new(
                        "mounted-fixture-tool",
                        arguments,
                    ))
                    .output(AdapterOutput::writable_private(
                        "initialized",
                        ".trail-generated/plugin-mounted",
                        ".trail-generated/plugin-mounted",
                    ))
                    .stale_reason(
                        "pinned plugin inputs, mounted action, executable, platform, or adapter changed",
                    )
                    .build()
                {
                    Ok(plan) => AdapterResult::PlannedV2 { plan },
                    Err(error) => AdapterResult::Error {
                        code: "invalid_plan".to_string(),
                        message: error.to_string(),
                    },
                }
            }
        };
        AdapterResponse::for_request(&request, result)
    }) {
        eprintln!("mounted-initializer-adapter: {error}");
        std::process::exit(1);
    }
}
