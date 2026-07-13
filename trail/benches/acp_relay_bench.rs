use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use trail::acp::{benchmark_acp_relay_frames, AcpRelayOptions};
use trail::{InitImportMode, Trail};

const FRAME_COUNT: usize = 10_000;

fn percentile(sorted: &[u128], numerator: usize, denominator: usize) -> u128 {
    let index = ((sorted.len() - 1) * numerator) / denominator;
    sorted[index]
}

fn main() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "ACP relay benchmark\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let options = AcpRelayOptions {
        workspace_root: temp.path().to_path_buf(),
        db_dir: temp.path().join(".trail"),
        lane: None,
        from_ref: None,
        provider: Some("benchmark".to_string()),
        model: None,
        materialize: false,
        workdir: None,
        inject_mcp: false,
        upstream_command: vec!["benchmark-agent".to_string()],
        upstream_env: BTreeMap::new(),
    };

    let frames = (0..FRAME_COUNT)
        .map(|sequence| {
            let (agent_to_client, value) = match sequence {
                0 => (false, serde_json::json!({"jsonrpc":"2.0","id":"benchmark-init","method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}})),
                1 => (true, serde_json::json!({"jsonrpc":"2.0","id":"benchmark-init","result":{"protocolVersion":1,"agentCapabilities":{}}})),
                _ if sequence % 4 == 0 => (false, serde_json::json!({"jsonrpc":"2.0","id":sequence as i64,"method":"ext/request","params":{"sequence":sequence}})),
                _ if sequence % 4 == 1 => (true, serde_json::json!({"jsonrpc":"2.0","method":"ext/update","params":{"sequence":sequence}})),
                _ if sequence % 4 == 2 => (true, serde_json::json!({"jsonrpc":"2.0","id":sequence as i64,"method":"ext/callback","params":{"sequence":sequence}})),
                _ => (false, serde_json::json!({"jsonrpc":"2.0","id":sequence as i64 - 1,"result":{"sequence":sequence}})),
            };
            let mut raw = serde_json::to_vec(&value).unwrap();
            raw.push(b'\n');
            (agent_to_client, raw)
        })
        .collect::<Vec<_>>();
    let expected = frames
        .iter()
        .map(|(_, raw)| raw.clone())
        .collect::<Vec<_>>();
    let samples = benchmark_acp_relay_frames(options, frames).unwrap();
    assert_eq!(samples.len(), FRAME_COUNT);
    for (index, sample) in samples.iter().enumerate() {
        if !sample.transformed {
            assert_eq!(sample.forwarded, expected[index], "frame {index} changed");
        }
    }

    let deadline = Instant::now() + Duration::from_secs(30);
    let receipts = loop {
        let db = Trail::open(temp.path()).unwrap();
        let mut receipts = Vec::new();
        for offset in (0..FRAME_COUNT).step_by(1_000) {
            receipts.extend(
                db.list_agent_hook_receipts_page(Some("trail-acp"), None, offset, 1_000)
                    .unwrap(),
            );
        }
        if receipts.len() == FRAME_COUNT {
            break receipts;
        }
        assert!(
            Instant::now() < deadline,
            "captured {} of {FRAME_COUNT} benchmark frames",
            receipts.len()
        );
        thread::sleep(Duration::from_millis(50));
    };
    let sequences = receipts
        .iter()
        .map(|receipt| receipt.connection_sequence.unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(sequences.len(), FRAME_COUNT);

    let mut latencies = samples
        .iter()
        .map(|sample| sample.latency_micros)
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    println!(
        "ACP relay forwarding latency: p50={}us p95={}us p99={}us frames={FRAME_COUNT}",
        percentile(&latencies, 50, 100),
        percentile(&latencies, 95, 100),
        percentile(&latencies, 99, 100),
    );
}
