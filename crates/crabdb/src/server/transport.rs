use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

use serde::Deserialize;

use crate::{CrabDb, Error, Result};

use super::route;

#[derive(Debug)]
pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct ServerAuth {
    pub(crate) token: Option<String>,
}

impl ServerAuth {
    pub fn disabled() -> Self {
        Self { token: None }
    }

    pub fn bearer(token: impl Into<String>) -> Result<Self> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(Error::InvalidInput(
                "daemon auth token cannot be empty".to_string(),
            ));
        }
        Ok(Self { token: Some(token) })
    }

    pub fn is_required(&self) -> bool {
        self.token.is_some()
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub(crate) reason: &'static str,
    pub(crate) body: Vec<u8>,
}

pub fn serve_listener(
    mut db: CrabDb,
    listener: TcpListener,
    max_requests: Option<usize>,
) -> Result<()> {
    serve_listener_with_auth(&mut db, listener, max_requests, ServerAuth::disabled())
}

pub fn serve_listener_with_auth(
    db: &mut CrabDb,
    listener: TcpListener,
    max_requests: Option<usize>,
    auth: ServerAuth,
) -> Result<()> {
    let mut handled = 0usize;
    loop {
        if max_requests.is_some_and(|max| handled >= max) {
            break;
        }
        let (stream, _) = listener.accept()?;
        handle_connection(db, stream, &auth)?;
        handled += 1;
    }
    Ok(())
}

pub fn handle_http_request(db: &mut CrabDb, raw: &[u8]) -> HttpResponse {
    handle_http_request_with_auth(db, raw, &ServerAuth::disabled())
}

pub fn handle_http_request_with_auth(
    db: &mut CrabDb,
    raw: &[u8],
    auth: &ServerAuth,
) -> HttpResponse {
    match parse_request(raw) {
        Ok(request) => route::route_request(db, request, auth),
        Err(err) => route::error_response(&err),
    }
}

fn handle_connection(db: &mut CrabDb, mut stream: TcpStream, auth: &ServerAuth) -> Result<()> {
    let request = read_request(&mut stream)?;
    let response = route::route_request(db, request, auth);
    stream.write_all(&response.to_http_bytes())?;
    stream.flush()?;
    Ok(())
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;
    if first_line.trim().is_empty() {
        return Err(Error::InvalidInput("empty HTTP request".to_string()));
    }
    let mut content_length = 0usize;
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().map_err(|_| {
                    Error::InvalidInput("invalid Content-Length header".to_string())
                })?;
            }
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    parse_request_parts(&first_line, headers, body)
}

fn parse_request(raw: &[u8]) -> Result<HttpRequest> {
    let raw = String::from_utf8_lossy(raw);
    let Some((head, body)) = raw.split_once("\r\n\r\n") else {
        return Err(Error::InvalidInput("malformed HTTP request".to_string()));
    };
    let mut lines = head.lines();
    let first_line = lines
        .next()
        .ok_or_else(|| Error::InvalidInput("empty HTTP request".to_string()))?;
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    parse_request_parts(first_line, headers, body.as_bytes().to_vec())
}

fn parse_request_parts(
    first_line: &str,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
) -> Result<HttpRequest> {
    let mut parts = first_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| Error::InvalidInput("missing HTTP method".to_string()))?;
    let path = parts
        .next()
        .ok_or_else(|| Error::InvalidInput("missing HTTP path".to_string()))?;
    Ok(HttpRequest {
        method: method.to_string(),
        path: path.to_string(),
        headers,
        body,
    })
}

impl HttpResponse {
    pub fn body_json<T: for<'de> Deserialize<'de>>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(Error::from)
    }

    pub(crate) fn to_http_bytes(&self) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.status,
            self.reason,
            self.body.len()
        )
        .into_bytes();
        out.extend_from_slice(&self.body);
        out
    }
}
