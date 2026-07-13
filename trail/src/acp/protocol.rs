use std::io;

use serde_json::Value;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Direction {
    ClientToAgent,
    AgentToClient,
}

impl Direction {
    fn opposite(self) -> Self {
        match self {
            Self::ClientToAgent => Self::AgentToClient,
            Self::AgentToClient => Self::ClientToAgent,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RequestId {
    Null,
    Number(i64),
    String(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CorrelationKey {
    pub direction: Direction,
    pub id: RequestId,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnvelopeKind {
    Request,
    Notification,
    SuccessResponse,
    ErrorResponse,
}

#[allow(dead_code)]
pub(crate) struct Frame {
    direction: Direction,
    raw: Vec<u8>,
    parsed: Value,
    kind: EnvelopeKind,
    method: Option<String>,
    id: Option<RequestId>,
    transformed: Option<Vec<u8>>,
}

#[allow(dead_code)]
impl Frame {
    pub(crate) fn parse(direction: Direction, raw: Vec<u8>) -> io::Result<Self> {
        let parsed: Value = serde_json::from_slice(&raw)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let (kind, method, id) = classify(&parsed)?;
        Ok(Self {
            direction,
            raw,
            parsed,
            kind,
            method,
            id,
            transformed: None,
        })
    }

    pub(crate) fn raw_bytes(&self) -> &[u8] {
        &self.raw
    }

    pub(crate) fn forward_bytes(&self) -> &[u8] {
        self.transformed.as_deref().unwrap_or(&self.raw)
    }

    pub(crate) fn kind(&self) -> EnvelopeKind {
        self.kind
    }

    pub(crate) fn direction(&self) -> Direction {
        self.direction
    }

    pub(crate) fn method(&self) -> Option<&str> {
        self.method.as_deref()
    }

    pub(crate) fn value(&self) -> &Value {
        &self.parsed
    }

    pub(crate) fn value_mut_for_transform(&mut self) -> &mut Value {
        &mut self.parsed
    }

    pub(crate) fn commit_transform(&mut self) -> io::Result<()> {
        let (kind, method, id) = classify(&self.parsed)?;
        if kind != self.kind || method != self.method || id != self.id {
            return Err(invalid(
                "ACP transformations must not change JSON-RPC routing fields",
            ));
        }
        let mut transformed = serde_json::to_vec(&self.parsed)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if self.raw.ends_with(b"\r\n") {
            transformed.extend_from_slice(b"\r\n");
        } else if self.raw.ends_with(b"\n") {
            transformed.push(b'\n');
        }
        self.transformed = Some(transformed);
        Ok(())
    }

    pub(crate) fn replace_value_and_commit(&mut self, candidate: Value) -> io::Result<()> {
        let original = std::mem::replace(&mut self.parsed, candidate);
        if let Err(error) = self.commit_transform() {
            self.parsed = original;
            self.transformed = None;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn correlation_key(&self) -> Option<CorrelationKey> {
        self.id.clone().map(|id| CorrelationKey {
            direction: match self.kind {
                EnvelopeKind::Request | EnvelopeKind::Notification => self.direction,
                EnvelopeKind::SuccessResponse | EnvelopeKind::ErrorResponse => {
                    self.direction.opposite()
                }
            },
            id,
        })
    }
}

fn classify(value: &Value) -> io::Result<(EnvelopeKind, Option<String>, Option<RequestId>)> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("JSON-RPC frame must be an object"))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(invalid("JSON-RPC frame must declare version 2.0"));
    }

    if let Some(method) = object.get("method") {
        let method = method
            .as_str()
            .ok_or_else(|| invalid("JSON-RPC method must be a string"))?;
        if object.contains_key("result") || object.contains_key("error") {
            return Err(invalid(
                "JSON-RPC request and notification envelopes cannot contain result or error",
            ));
        }
        let id = object.get("id").map(request_id).transpose()?;
        let kind = if id.is_some() {
            EnvelopeKind::Request
        } else {
            EnvelopeKind::Notification
        };
        return Ok((kind, Some(method.to_string()), id));
    }

    let id = request_id(
        object
            .get("id")
            .ok_or_else(|| invalid("JSON-RPC response must have an id"))?,
    )?;
    let has_result = object.contains_key("result");
    let has_error = object.contains_key("error");
    match (has_result, has_error) {
        (true, false) => Ok((EnvelopeKind::SuccessResponse, None, Some(id))),
        (false, true) => {
            validate_error_object(&object["error"])?;
            Ok((EnvelopeKind::ErrorResponse, None, Some(id)))
        }
        (true, true) => Err(invalid(
            "JSON-RPC response cannot contain both result and error",
        )),
        (false, false) => Err(invalid(
            "JSON-RPC response must contain exactly one of result or error",
        )),
    }
}

fn validate_error_object(value: &Value) -> io::Result<()> {
    let error = value
        .as_object()
        .ok_or_else(|| invalid("JSON-RPC error must be an object"))?;
    if error.get("code").and_then(Value::as_i64).is_none() {
        return Err(invalid("JSON-RPC error code must be an integer"));
    }
    if error.get("message").and_then(Value::as_str).is_none() {
        return Err(invalid("JSON-RPC error message must be a string"));
    }
    Ok(())
}

fn request_id(value: &Value) -> io::Result<RequestId> {
    match value {
        Value::Null => Ok(RequestId::Null),
        Value::Number(number) => number
            .as_i64()
            .map(RequestId::Number)
            .ok_or_else(|| invalid("JSON-RPC numeric ids must be signed 64-bit integers")),
        Value::String(value) => Ok(RequestId::String(value.clone())),
        _ => Err(invalid("JSON-RPC id must be null, an integer, or a string")),
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_preserves_raw_bytes_and_scopes_ids_by_direction() {
        let client_raw = br#" { "id":7, "jsonrpc":"2.0", "method":"session/list", "params":{} }
"#
        .to_vec();
        let agent_raw = br#" { "id":7, "jsonrpc":"2.0", "method":"fs/read_text_file", "params":{"sessionId":"s","path":"a"} }
"#
        .to_vec();
        let client = Frame::parse(Direction::ClientToAgent, client_raw.clone()).unwrap();
        let agent = Frame::parse(Direction::AgentToClient, agent_raw.clone()).unwrap();

        assert_eq!(client.kind(), EnvelopeKind::Request);
        assert_eq!(agent.kind(), EnvelopeKind::Request);
        assert_eq!(client.forward_bytes(), client_raw);
        assert_eq!(agent.forward_bytes(), agent_raw);
        assert_ne!(client.correlation_key(), agent.correlation_key());
    }

    #[test]
    fn classifies_every_json_rpc_envelope_and_scopes_response_ids() {
        let request = Frame::parse(
            Direction::ClientToAgent,
            br#"{"jsonrpc":"2.0","id":"same","method":"session/list"}
"#
            .to_vec(),
        )
        .unwrap();
        let notification = Frame::parse(
            Direction::ClientToAgent,
            br#"{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"s"}}
"#
            .to_vec(),
        )
        .unwrap();
        let success = Frame::parse(
            Direction::AgentToClient,
            br#"{"jsonrpc":"2.0","id":"same","result":{"sessions":[]}}
"#
            .to_vec(),
        )
        .unwrap();
        let error = Frame::parse(
            Direction::AgentToClient,
            br#"{"jsonrpc":"2.0","id":9,"error":{"code":-32602,"message":"bad params"}}
"#
            .to_vec(),
        )
        .unwrap();

        assert_eq!(request.kind(), EnvelopeKind::Request);
        assert_eq!(notification.kind(), EnvelopeKind::Notification);
        assert_eq!(success.kind(), EnvelopeKind::SuccessResponse);
        assert_eq!(error.kind(), EnvelopeKind::ErrorResponse);
        assert_eq!(request.method(), Some("session/list"));
        assert_eq!(notification.method(), Some("session/cancel"));
        assert_eq!(success.method(), None);
        assert_eq!(notification.correlation_key(), None);
        assert_eq!(success.correlation_key(), request.correlation_key());
    }

    #[test]
    fn rejects_invalid_json_rpc_envelopes() {
        let invalid = [
            br#"[]"#.as_slice(),
            br#"{"id":1,"method":"session/list"}"#.as_slice(),
            br#"{"jsonrpc":"1.0","id":1,"method":"session/list"}"#.as_slice(),
            br#"{"jsonrpc":"2.0","id":1.5,"method":"session/list"}"#.as_slice(),
            br#"{"jsonrpc":"2.0","id":{},"method":"session/list"}"#.as_slice(),
            br#"{"jsonrpc":"2.0","id":1,"method":7}"#.as_slice(),
            br#"{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":-1,"message":"x"}}"#.as_slice(),
            br#"{"jsonrpc":"2.0","result":{}}"#.as_slice(),
            br#"{"jsonrpc":"2.0","id":1}"#.as_slice(),
        ];
        for raw in invalid {
            assert!(
                Frame::parse(Direction::ClientToAgent, raw.to_vec()).is_err(),
                "accepted invalid envelope: {}",
                String::from_utf8_lossy(raw)
            );
        }
    }

    #[test]
    fn transformation_is_explicit_and_preserves_the_original_line_ending() {
        let raw = b" {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1}}\r\n".to_vec();
        let mut frame = Frame::parse(Direction::ClientToAgent, raw.clone()).unwrap();
        frame.value_mut_for_transform()["params"]["clientCapabilities"] = serde_json::json!({});

        assert_eq!(frame.forward_bytes(), raw);
        frame.commit_transform().unwrap();
        assert_ne!(frame.forward_bytes(), raw);
        assert!(frame.forward_bytes().ends_with(b"\r\n"));
        assert_eq!(
            frame.value()["params"]["clientCapabilities"],
            serde_json::json!({})
        );
    }

    #[test]
    fn pinned_message_fixture_covers_all_envelope_kinds() {
        let kinds = include_str!("../../tests/fixtures/acp/v1/messages.jsonl")
            .lines()
            .map(|line| {
                Frame::parse(Direction::ClientToAgent, format!("{line}\n").into_bytes())
                    .unwrap()
                    .kind()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                EnvelopeKind::Request,
                EnvelopeKind::Notification,
                EnvelopeKind::SuccessResponse,
                EnvelopeKind::ErrorResponse,
            ]
        );
    }
}
