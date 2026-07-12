use std::io::{self, Write};
use std::process::Command;
use std::time::Duration;

use trail_environment_adapter_sdk::{
    read_frame, AdapterRequest, AdapterResponse, AdapterResult, MAX_FRAME_BYTES, PROTOCOL_V1,
};

fn main() {
    let request: AdapterRequest = match read_frame(&mut io::stdin().lock(), MAX_FRAME_BYTES) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("adversarial-adapter could not read request: {error}");
            std::process::exit(2);
        }
    };
    match request.adapter_identity.as_str() {
        "example/hang@1" => {
            std::thread::sleep(Duration::from_secs(10));
            let _ = trail_environment_adapter_sdk::write_frame(
                &mut io::stdout().lock(),
                &AdapterResponse {
                    protocol: PROTOCOL_V1.to_string(),
                    request_id: request.request_id,
                    result: AdapterResult::Discovered { component: None },
                },
                MAX_FRAME_BYTES,
            );
        }
        "example/crash@1" => std::process::exit(7),
        "example/oversized@1" => {
            let block = vec![b'x'; 2 * 1024 * 1024];
            let _ = io::stdout().lock().write_all(&block);
        }
        "example/malformed@1" => {
            let _ = io::stdout().lock().write_all(b"not a framed response");
        }
        "example/child@1" => {
            if Command::new(std::env::current_exe().unwrap())
                .spawn()
                .is_ok()
            {
                let _ = trail_environment_adapter_sdk::write_frame(
                    &mut io::stdout().lock(),
                    &AdapterResponse {
                        protocol: PROTOCOL_V1.to_string(),
                        request_id: request.request_id,
                        result: AdapterResult::Discovered {
                            component: Some(trail_environment_adapter_sdk::DiscoveredComponent {
                                component_id: "plugin.child-escaped".to_string(),
                                kind: "generated".to_string(),
                            }),
                        },
                    },
                    MAX_FRAME_BYTES,
                );
            } else {
                std::process::exit(9);
            }
        }
        "example/memory@1" => {
            let mut allocation = Vec::<Vec<u8>>::new();
            for _ in 0..80 {
                allocation.push(vec![0xa5; 8 * 1024 * 1024]);
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = allocation.len();
            std::process::exit(10);
        }
        _ => std::process::exit(3),
    }
}
