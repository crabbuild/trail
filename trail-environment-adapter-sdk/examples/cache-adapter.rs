use trail_environment_adapter_sdk::{
    serve_once, AdapterCache, AdapterCacheProtocol, AdapterCommand, AdapterOperation,
    AdapterOutput, AdapterPlanV2, AdapterResponse, AdapterResult, DiscoveredComponent, PROTOCOL_V2,
};

fn main() {
    if let Err(error) = serve_once(|request| {
        if request.protocol != PROTOCOL_V2 || request.adapter_identity != "example/cache@1" {
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
                    .any(|file| file.path == "cache.adapter")
                    .then(|| DiscoveredComponent::new("plugin.cache", "generated")),
            },
            AdapterOperation::Plan {
                component_id,
                files,
                ..
            } => {
                let behavior = files
                    .iter()
                    .find(|file| file.path == "cache.adapter")
                    .map(|file| String::from_utf8_lossy(&file.content).trim().to_string())
                    .unwrap_or_else(|| "populate".to_string());
                match AdapterPlanV2::builder(component_id.clone(), "generated")
                    .identity_input("cache.adapter")
                    .semantic_input("fixture", "host-exclusive-cache-v1")
                    .cache(
                        AdapterCache::host_exclusive(
                            "fixture-store",
                            AdapterCacheProtocol::ContentStore,
                        )
                        .compatibility_dimension("fixture_tool", "cache-fixture-tool@1")
                        .environment_variable("TRAIL_FIXTURE_CACHE", "."),
                    )
                    .staging_command(AdapterCommand::new("cache-fixture-tool", [behavior]))
                    .output(AdapterOutput::immutable_seed_private(
                        "generated",
                        "generated",
                        ".trail-generated/plugin-cache",
                    ))
                    .stale_reason("cache fixture input, executable, platform, or adapter changed")
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
        eprintln!("cache-adapter: {error}");
        std::process::exit(1);
    }
}
