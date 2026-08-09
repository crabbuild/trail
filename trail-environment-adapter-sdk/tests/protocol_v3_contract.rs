use std::collections::BTreeSet;

use proptest::prelude::*;
use trail_environment_adapter_sdk::{
    negotiate_highest_mutual_protocol, read_frame, write_frame, AdapterComponentProposalV3,
    AdapterHost, AdapterOperationV3, AdapterPackageCapabilities, AdapterPackageManifest,
    AdapterProposalStatusV3, AdapterRequestV3, AdapterResponseV3, AdapterResultV3, PinnedFile,
    ProtocolError, PROTOCOL_V1, PROTOCOL_V2, PROTOCOL_V3,
};

const GOLDEN_REQUEST_FRAME: &str = include_str!("fixtures/protocol-v3-propose-request.frame.hex");
const GOLDEN_RESPONSE_FRAME: &str = include_str!("fixtures/protocol-v3-propose-response.frame.hex");

fn proposal() -> AdapterComponentProposalV3 {
    AdapterComponentProposalV3 {
        component_id: "fixture.codegen".into(),
        component_root: ".".into(),
        kind: "generated".into(),
        status: AdapterProposalStatusV3::Ready,
        proposal_key: "sha256:fixture-proposal".into(),
        missing_requirements: Vec::new(),
        recovery_actions: Vec::new(),
    }
}

fn request() -> AdapterRequestV3 {
    AdapterRequestV3::new(
        "fixture-request",
        "fixture/codegen@1",
        "sha256:fixture-distribution",
        AdapterHost {
            operating_system: "linux".into(),
            architecture: "x86_64".into(),
        },
        "root:fixture",
        AdapterOperationV3::Propose {
            component_root: ".".into(),
            files: vec![PinnedFile {
                path: "schema.json".into(),
                content_hash: "sha256:fixture-input".into(),
                executable: false,
                content: b"{}\n".to_vec(),
            }],
        },
    )
}

fn response() -> AdapterResponseV3 {
    AdapterResponseV3::for_request(
        &request(),
        AdapterResultV3::Proposed {
            component: Some(proposal()),
        },
    )
}

fn framed<T: serde::Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, value, 1024 * 1024).unwrap();
    bytes
}

fn golden_bytes(fixture: &str) -> Vec<u8> {
    hex::decode(fixture.split_whitespace().collect::<String>()).unwrap()
}

#[test]
fn protocol_v3_golden_frames_are_stable_and_round_trip() {
    let request = request();
    let request_frame = framed(&request);
    assert_eq!(
        request_frame,
        golden_bytes(GOLDEN_REQUEST_FRAME),
        "request frame changed: {}",
        hex::encode(&request_frame)
    );
    let decoded: AdapterRequestV3 = read_frame(
        &mut golden_bytes(GOLDEN_REQUEST_FRAME).as_slice(),
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(decoded, request);

    let response = response();
    let response_frame = framed(&response);
    assert_eq!(
        response_frame,
        golden_bytes(GOLDEN_RESPONSE_FRAME),
        "response frame changed: {}",
        hex::encode(&response_frame)
    );
    let decoded: AdapterResponseV3 = read_frame(
        &mut golden_bytes(GOLDEN_RESPONSE_FRAME).as_slice(),
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(decoded, response);
}

#[test]
fn protocol_v3_frames_fail_closed_on_truncation_and_declared_over_limit() {
    let complete = framed(&request());
    for length in [0, 1, 3, 4, complete.len() - 1] {
        let mut truncated = &complete[..length];
        assert!(matches!(
            read_frame::<AdapterRequestV3>(&mut truncated, 1024 * 1024),
            Err(ProtocolError::Truncated)
        ));
    }

    let body_length = u32::from_be_bytes(complete[..4].try_into().unwrap()) as usize;
    assert!(matches!(
        read_frame::<AdapterRequestV3>(&mut complete.as_slice(), body_length - 1),
        Err(ProtocolError::FrameTooLarge {
            actual,
            maximum
        }) if actual == body_length && maximum == body_length - 1
    ));
    assert!(matches!(
        write_frame(&mut Vec::new(), &request(), body_length - 1),
        Err(ProtocolError::FrameTooLarge {
            actual,
            maximum
        }) if actual == body_length && maximum == body_length - 1
    ));
}

#[test]
fn legacy_protocol_lists_negotiate_without_v3_authority() {
    let legacy_v1 = vec![PROTOCOL_V1.to_string()];
    let legacy_v2 = vec![PROTOCOL_V1.to_string(), PROTOCOL_V2.to_string()];
    assert_eq!(
        negotiate_highest_mutual_protocol(&[PROTOCOL_V3, PROTOCOL_V2, PROTOCOL_V1], &legacy_v1),
        Some(PROTOCOL_V1)
    );
    assert_eq!(
        negotiate_highest_mutual_protocol(&[PROTOCOL_V3, PROTOCOL_V2, PROTOCOL_V1], &legacy_v2),
        Some(PROTOCOL_V2)
    );
}

#[test]
fn legacy_package_fixtures_preserve_exact_v1_v2_meaning() {
    let v1: AdapterPackageManifest = toml::from_str(include_str!(
        "fixtures/legacy-v1-package-without-protocols.toml"
    ))
    .unwrap();
    assert_eq!(v1.adapter.protocols, [PROTOCOL_V1]);
    assert_eq!(
        v1.adapter.capabilities,
        AdapterPackageCapabilities::default()
    );

    let v2: AdapterPackageManifest = toml::from_str(include_str!(
        "fixtures/legacy-v2-package-without-capabilities.toml"
    ))
    .unwrap();
    assert_eq!(v2.adapter.protocols, [PROTOCOL_V2, PROTOCOL_V1]);
    assert_eq!(
        v2.adapter.capabilities,
        AdapterPackageCapabilities::default()
    );
    assert!(!v2
        .adapter
        .protocols
        .iter()
        .any(|value| value == PROTOCOL_V3));
}

proptest! {
    #[test]
    fn negotiation_is_exact_order_independent_and_selects_the_highest_mutual(
        host in prop::collection::vec(0u8..6, 0..12),
        package in prop::collection::vec(0u8..6, 0..12),
    ) {
        const IDENTITIES: [&str; 6] = [
            PROTOCOL_V1,
            PROTOCOL_V2,
            PROTOCOL_V3,
            "trail.environment-adapter/v3-preview",
            "trail.environment-adapter/v30",
            "unknown",
        ];
        let host = host.into_iter().map(|index| IDENTITIES[index as usize]).collect::<Vec<_>>();
        let package = package
            .into_iter()
            .map(|index| IDENTITIES[index as usize].to_string())
            .collect::<Vec<_>>();
        let host_set = host.iter().copied().collect::<BTreeSet<_>>();
        let package_set = package.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let expected = [PROTOCOL_V3, PROTOCOL_V2, PROTOCOL_V1]
            .into_iter()
            .find(|candidate| host_set.contains(candidate) && package_set.contains(candidate));
        prop_assert_eq!(negotiate_highest_mutual_protocol(&host, &package), expected);

        let mut reversed_host = host;
        let mut reversed_package = package;
        reversed_host.reverse();
        reversed_package.reverse();
        prop_assert_eq!(
            negotiate_highest_mutual_protocol(&reversed_host, &reversed_package),
            expected
        );
    }
}
