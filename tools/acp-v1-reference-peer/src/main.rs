use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use agent_client_protocol_schema::v1::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

type PeerResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn typed<T: DeserializeOwned>(value: Value) -> PeerResult<T> {
    Ok(serde_json::from_value(value)?)
}

fn send<T: Serialize>(writer: &mut impl Write, message: &T) -> PeerResult {
    serde_json::to_writer(&mut *writer, message)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn read_value(reader: &mut impl BufRead) -> PeerResult<Value> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err("peer closed before the next ACP frame".into());
    }
    Ok(serde_json::from_str(&line)?)
}

fn receive<T: DeserializeOwned>(reader: &mut impl BufRead) -> PeerResult<T> {
    typed(read_value(reader)?)
}

fn typed_send<T: DeserializeOwned + Serialize>(
    writer: &mut impl Write,
    value: Value,
) -> PeerResult {
    send(writer, &typed::<T>(value)?)
}

fn response_id(value: &Value) -> Value {
    value.get("id").cloned().unwrap_or(Value::Null)
}

fn run_agent() -> PeerResult {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let mut cwd = String::new();
    let session_id = "official-session".to_string();

    while let Ok(message) = read_value(&mut input) {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = response_id(&message);
        match method {
            "initialize" => {
                let _: JsonRpcMessage<Request<InitializeRequest>> = typed(message)?;
                typed_send::<JsonRpcMessage<Response<InitializeResponse>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"list":{},"delete":{},"resume":{},"close":{}}}}}),
                )?;
            }
            "authenticate" => {
                let _: JsonRpcMessage<Request<AuthenticateRequest>> = typed(message)?;
                typed_send::<JsonRpcMessage<Response<AuthenticateResponse>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"reference auth rejection","data":{"typed":true}}}),
                )?;
            }
            "session/new" => {
                let _: JsonRpcMessage<Request<NewSessionRequest>> = typed(message.clone())?;
                cwd = message["params"]["cwd"].as_str().unwrap().to_string();
                typed_send::<JsonRpcMessage<Response<NewSessionResponse>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":session_id}}),
                )?;
            }
            "session/load" => {
                let _: JsonRpcMessage<Request<LoadSessionRequest>> = typed(message)?;
                typed_send::<JsonRpcMessage<Response<LoadSessionResponse>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"result":{}}),
                )?;
            }
            "session/resume" => {
                let _: JsonRpcMessage<Request<ResumeSessionRequest>> = typed(message)?;
                typed_send::<JsonRpcMessage<Response<ResumeSessionResponse>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"result":{}}),
                )?;
            }
            "session/close" => {
                let _: JsonRpcMessage<Request<CloseSessionRequest>> = typed(message)?;
                typed_send::<JsonRpcMessage<Response<CloseSessionResponse>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"result":{}}),
                )?;
            }
            "session/delete" => {
                let _: JsonRpcMessage<Request<DeleteSessionRequest>> = typed(message)?;
                typed_send::<JsonRpcMessage<Response<DeleteSessionResponse>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"result":{}}),
                )?;
            }
            "logout" => {
                let _: JsonRpcMessage<Request<LogoutRequest>> = typed(message)?;
                typed_send::<JsonRpcMessage<Response<LogoutResponse>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"result":{}}),
                )?;
            }
            "session/prompt" => {
                let _: JsonRpcMessage<Request<PromptRequest>> = typed(message)?;
                let cancellation: JsonRpcMessage<Notification<CancelNotification>> =
                    receive(&mut input)?;
                let _ = cancellation;
                let rpc_cancellation: JsonRpcMessage<Notification<CancelRequestNotification>> =
                    receive(&mut input)?;
                let _ = rpc_cancellation;

                typed_send::<JsonRpcMessage<Notification<SessionNotification>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session_id,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"official update"}}}}),
                )?;
                callback::<RequestPermissionRequest, RequestPermissionResponse>(
                    &mut input,
                    &mut output,
                    json!({"jsonrpc":"2.0","id":"permission","method":"session/request_permission","params":{"sessionId":session_id,"toolCall":{"toolCallId":"official-tool","title":"Official permission"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]}}),
                )?;
                callback::<ReadTextFileRequest, ReadTextFileResponse>(
                    &mut input,
                    &mut output,
                    json!({"jsonrpc":"2.0","id":"read","method":"fs/read_text_file","params":{"sessionId":session_id,"path":format!("{cwd}/README.md")}}),
                )?;
                callback::<WriteTextFileRequest, WriteTextFileResponse>(
                    &mut input,
                    &mut output,
                    json!({"jsonrpc":"2.0","id":"write","method":"fs/write_text_file","params":{"sessionId":session_id,"path":format!("{cwd}/official.txt"),"content":"official"}}),
                )?;
                callback::<CreateTerminalRequest, CreateTerminalResponse>(
                    &mut input,
                    &mut output,
                    json!({"jsonrpc":"2.0","id":"create","method":"terminal/create","params":{"sessionId":session_id,"command":"printf","args":["official"],"cwd":cwd}}),
                )?;
                for (request_id, method_name) in [
                    ("output", "terminal/output"),
                    ("wait", "terminal/wait_for_exit"),
                    ("kill", "terminal/kill"),
                    ("release", "terminal/release"),
                ] {
                    terminal_callback(
                        &mut input,
                        &mut output,
                        request_id,
                        method_name,
                        &session_id,
                    )?;
                }
                typed_send::<JsonRpcMessage<Response<PromptResponse>>>(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"cancelled"}}),
                )?;
            }
            _ => return Err(format!("official agent received unexpected method {method}").into()),
        }
    }
    Ok(())
}

fn callback<P, R>(input: &mut impl BufRead, output: &mut impl Write, request: Value) -> PeerResult
where
    P: DeserializeOwned + Serialize,
    R: DeserializeOwned,
{
    typed_send::<JsonRpcMessage<Request<P>>>(output, request)?;
    let _: JsonRpcMessage<Response<R>> = receive(input)?;
    Ok(())
}

fn terminal_callback(
    input: &mut impl BufRead,
    output: &mut impl Write,
    id: &str,
    method: &str,
    session_id: &str,
) -> PeerResult {
    let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":{"sessionId":session_id,"terminalId":"official-terminal"}});
    match method {
        "terminal/output" => {
            callback::<TerminalOutputRequest, TerminalOutputResponse>(input, output, request)
        }
        "terminal/wait_for_exit" => callback::<
            WaitForTerminalExitRequest,
            WaitForTerminalExitResponse,
        >(input, output, request),
        "terminal/kill" => {
            callback::<KillTerminalRequest, KillTerminalResponse>(input, output, request)
        }
        "terminal/release" => {
            callback::<ReleaseTerminalRequest, ReleaseTerminalResponse>(input, output, request)
        }
        _ => unreachable!(),
    }
}

struct RelayClient {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl RelayClient {
    fn spawn(workspace: &Path, trail: &Path, agent: &Path) -> PeerResult<Self> {
        let mut child = Command::new(trail)
            .arg("--workspace")
            .arg(workspace)
            .args(["acp", "relay", "--"])
            .arg(agent)
            .arg("agent")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = child.stdin.take().ok_or("missing relay stdin")?;
        let output = BufReader::new(child.stdout.take().ok_or("missing relay stdout")?);
        Ok(Self {
            child,
            input,
            output,
        })
    }

    fn exchange<P, R>(&mut self, request: Value) -> PeerResult<JsonRpcMessage<Response<R>>>
    where
        P: DeserializeOwned + Serialize,
        R: DeserializeOwned,
    {
        typed_send::<JsonRpcMessage<Request<P>>>(&mut self.input, request)?;
        receive(&mut self.output)
    }
}

fn run_client(workspace: &Path, trail: &Path, agent: &Path) -> PeerResult {
    let mut relay = RelayClient::spawn(workspace, trail, agent)?;
    let _: JsonRpcMessage<Response<InitializeResponse>> = relay.exchange::<InitializeRequest, _>(
        json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":true}}}),
    )?;
    let auth: JsonRpcMessage<Response<AuthenticateResponse>> = relay.exchange::<AuthenticateRequest, _>(
        json!({"jsonrpc":"2.0","id":"auth","method":"authenticate","params":{"methodId":"official"}}),
    )?;
    assert!(matches!(auth.inner(), Response::Error { error, .. } if error.data.is_some()));
    let session: JsonRpcMessage<Response<NewSessionResponse>> = relay.exchange::<NewSessionRequest, _>(
        json!({"jsonrpc":"2.0","id":"new","method":"session/new","params":{"cwd":workspace,"mcpServers":[]}}),
    )?;
    assert!(matches!(session.inner(), Response::Result { .. }));

    typed_send::<JsonRpcMessage<Request<PromptRequest>>>(
        &mut relay.input,
        json!({"jsonrpc":"2.0","id":"prompt","method":"session/prompt","params":{"sessionId":"official-session","prompt":[{"type":"text","text":"official prompt"},{"type":"resource_link","name":"README","uri":"file:///README.md"}]}}),
    )?;
    typed_send::<JsonRpcMessage<Notification<CancelNotification>>>(
        &mut relay.input,
        json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"official-session"}}),
    )?;
    typed_send::<JsonRpcMessage<Notification<CancelRequestNotification>>>(
        &mut relay.input,
        json!({"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"prompt"}}),
    )?;

    loop {
        let message = read_value(&mut relay.output)?;
        if message.get("method").and_then(Value::as_str) == Some("session/update") {
            let update: JsonRpcMessage<Notification<SessionNotification>> = typed(message)?;
            assert!(matches!(
                update.inner().params,
                Some(SessionNotification {
                    update: SessionUpdate::AgentMessageChunk(_),
                    ..
                })
            ));
            continue;
        }
        if let Some(method) = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            respond_to_callback(&mut relay.input, &method, message)?;
            continue;
        }
        let response: JsonRpcMessage<Response<PromptResponse>> = typed(message)?;
        assert!(
            matches!(response.inner(), Response::Result { result, .. } if result.stop_reason == StopReason::Cancelled)
        );
        break;
    }

    for (id, method, params, response_kind) in [
        (
            "load",
            "session/load",
            json!({"sessionId":"official-session","cwd":workspace,"mcpServers":[]}),
            "load",
        ),
        (
            "resume",
            "session/resume",
            json!({"sessionId":"official-session","cwd":workspace,"mcpServers":[]}),
            "resume",
        ),
        (
            "close",
            "session/close",
            json!({"sessionId":"official-session"}),
            "close",
        ),
        (
            "delete",
            "session/delete",
            json!({"sessionId":"official-session"}),
            "delete",
        ),
    ] {
        let value = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        match response_kind {
            "load" => {
                let _: JsonRpcMessage<Response<LoadSessionResponse>> =
                    relay.exchange::<LoadSessionRequest, _>(value)?;
            }
            "resume" => {
                let _: JsonRpcMessage<Response<ResumeSessionResponse>> =
                    relay.exchange::<ResumeSessionRequest, _>(value)?;
            }
            "close" => {
                let _: JsonRpcMessage<Response<CloseSessionResponse>> =
                    relay.exchange::<CloseSessionRequest, _>(value)?;
            }
            "delete" => {
                let _: JsonRpcMessage<Response<DeleteSessionResponse>> =
                    relay.exchange::<DeleteSessionRequest, _>(value)?;
            }
            _ => unreachable!(),
        }
    }
    let _: JsonRpcMessage<Response<LogoutResponse>> = relay.exchange::<LogoutRequest, _>(
        json!({"jsonrpc":"2.0","id":"logout","method":"logout","params":{}}),
    )?;
    drop(relay.input);
    let status = relay.child.wait()?;
    if !status.success() {
        return Err(format!("Trail relay exited with {status}").into());
    }
    Ok(())
}

fn run_basic_client(workspace: &Path, trail: &Path, agent: &Path) -> PeerResult {
    let mut relay = RelayClient::spawn(workspace, trail, agent)?;
    let _: JsonRpcMessage<Response<InitializeResponse>> = relay.exchange::<InitializeRequest, _>(
        json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}),
    )?;
    let _: JsonRpcMessage<Response<NewSessionResponse>> = relay.exchange::<NewSessionRequest, _>(
        json!({"jsonrpc":"2.0","id":"new","method":"session/new","params":{"cwd":workspace,"mcpServers":[]}}),
    )?;
    typed_send::<JsonRpcMessage<Request<PromptRequest>>>(
        &mut relay.input,
        json!({"jsonrpc":"2.0","id":"prompt","method":"session/prompt","params":{"sessionId":"fixture-session","prompt":[{"type":"text","text":"official client to fixture agent"}]}}),
    )?;
    let update: JsonRpcMessage<Notification<SessionNotification>> = receive(&mut relay.output)?;
    assert!(matches!(
        update.inner().params,
        Some(SessionNotification {
            update: SessionUpdate::AgentMessageChunk(_),
            ..
        })
    ));
    let response: JsonRpcMessage<Response<PromptResponse>> = receive(&mut relay.output)?;
    assert!(matches!(
        response.inner(),
        Response::Result { result, .. } if result.stop_reason == StopReason::EndTurn
    ));
    let _: JsonRpcMessage<Response<CloseSessionResponse>> = relay
        .exchange::<CloseSessionRequest, _>(json!({"jsonrpc":"2.0","id":"close","method":"session/close","params":{"sessionId":"fixture-session"}}))?;
    drop(relay.input);
    let status = relay.child.wait()?;
    if !status.success() {
        return Err(format!("Trail relay exited with {status}").into());
    }
    Ok(())
}

fn respond_to_callback(output: &mut impl Write, method: &str, message: Value) -> PeerResult {
    let id = response_id(&message);
    match method {
        "session/request_permission" => {
            let _: JsonRpcMessage<Request<RequestPermissionRequest>> = typed(message)?;
            typed_send::<JsonRpcMessage<Response<RequestPermissionResponse>>>(
                output,
                json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"selected","optionId":"allow"}}}),
            )
        }
        "fs/read_text_file" => {
            let _: JsonRpcMessage<Request<ReadTextFileRequest>> = typed(message)?;
            typed_send::<JsonRpcMessage<Response<ReadTextFileResponse>>>(
                output,
                json!({"jsonrpc":"2.0","id":id,"result":{"content":"official read"}}),
            )
        }
        "fs/write_text_file" => {
            let _: JsonRpcMessage<Request<WriteTextFileRequest>> = typed(message)?;
            typed_send::<JsonRpcMessage<Response<WriteTextFileResponse>>>(
                output,
                json!({"jsonrpc":"2.0","id":id,"result":{}}),
            )
        }
        "terminal/create" => {
            let _: JsonRpcMessage<Request<CreateTerminalRequest>> = typed(message)?;
            typed_send::<JsonRpcMessage<Response<CreateTerminalResponse>>>(
                output,
                json!({"jsonrpc":"2.0","id":id,"result":{"terminalId":"official-terminal"}}),
            )
        }
        "terminal/output" => {
            typed_callback_response::<TerminalOutputRequest, TerminalOutputResponse>(
                output,
                message,
                json!({"output":"official","truncated":false,"exitStatus":{"exitCode":0,"signal":null}}),
            )
        }
        "terminal/wait_for_exit" => typed_callback_response::<
            WaitForTerminalExitRequest,
            WaitForTerminalExitResponse,
        >(output, message, json!({"exitCode":0,"signal":null})),
        "terminal/kill" => typed_callback_response::<KillTerminalRequest, KillTerminalResponse>(
            output,
            message,
            json!({}),
        ),
        "terminal/release" => typed_callback_response::<
            ReleaseTerminalRequest,
            ReleaseTerminalResponse,
        >(output, message, json!({})),
        _ => Err(format!("official client received unexpected callback {method}").into()),
    }
}

fn typed_callback_response<P, R>(
    output: &mut impl Write,
    message: Value,
    result: Value,
) -> PeerResult
where
    P: DeserializeOwned,
    R: DeserializeOwned + Serialize,
{
    let id = response_id(&message);
    let _: JsonRpcMessage<Request<P>> = typed(message)?;
    let result = serde_json::to_value(typed::<R>(result)?)?;
    typed_send::<JsonRpcMessage<Response<R>>>(
        output,
        json!({"jsonrpc":"2.0","id":id,"result":result}),
    )
}

fn main() -> PeerResult {
    let args = std::env::args_os().collect::<Vec<_>>();
    match args.get(1).and_then(|arg| arg.to_str()) {
        Some("agent") => run_agent(),
        Some("client") if args.len() == 5 => run_client(
            Path::new(&args[2]),
            Path::new(&args[3]),
            Path::new(&args[4]),
        ),
        Some("client-basic") if args.len() == 5 => run_basic_client(
            Path::new(&args[2]),
            Path::new(&args[3]),
            Path::new(&args[4]),
        ),
        _ => Err(
            "usage: acp-v1-reference-peer agent | (client|client-basic) WORKSPACE TRAIL AGENT"
                .into(),
        ),
    }
}
