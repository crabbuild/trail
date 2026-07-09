use std::collections::BTreeMap;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use crate::{Error, Result, Trail};

use super::route;

const MAX_HTTP_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const HTTP_CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_RATE_LIMIT_REQUESTS: usize = 600;
const DEFAULT_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

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

#[derive(Clone, Debug)]
pub struct ServerRateLimit {
    max_requests: Option<usize>,
    window: Duration,
}

impl ServerRateLimit {
    pub fn per_window(max_requests: usize, window: Duration) -> Result<Self> {
        if max_requests == 0 {
            return Err(Error::InvalidInput(
                "rate limit max_requests must be greater than zero".to_string(),
            ));
        }
        if window.is_zero() {
            return Err(Error::InvalidInput(
                "rate limit window must be greater than zero".to_string(),
            ));
        }
        Ok(Self {
            max_requests: Some(max_requests),
            window,
        })
    }

    pub fn disabled() -> Self {
        Self {
            max_requests: None,
            window: DEFAULT_RATE_LIMIT_WINDOW,
        }
    }
}

impl Default for ServerRateLimit {
    fn default() -> Self {
        Self {
            max_requests: Some(DEFAULT_RATE_LIMIT_REQUESTS),
            window: DEFAULT_RATE_LIMIT_WINDOW,
        }
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub(crate) reason: &'static str,
    pub(crate) extra_headers: Vec<(&'static str, String)>,
    pub(crate) body: Vec<u8>,
}

pub fn serve_listener(
    mut db: Trail,
    listener: TcpListener,
    max_requests: Option<usize>,
) -> Result<()> {
    serve_listener_with_auth(&mut db, listener, max_requests, ServerAuth::disabled())
}

pub fn serve_listener_with_auth(
    db: &mut Trail,
    listener: TcpListener,
    max_requests: Option<usize>,
    auth: ServerAuth,
) -> Result<()> {
    serve_listener_with_auth_and_rate_limit(
        db,
        listener,
        max_requests,
        auth,
        ServerRateLimit::default(),
    )
}

pub fn serve_listener_with_auth_and_rate_limit(
    db: &mut Trail,
    listener: TcpListener,
    max_requests: Option<usize>,
    auth: ServerAuth,
    rate_limit: ServerRateLimit,
) -> Result<()> {
    serve_listener_with_auth_rate_limit_and_timeout(
        db,
        listener,
        max_requests,
        auth,
        rate_limit,
        HTTP_CONNECTION_TIMEOUT,
    )
}

pub fn serve_listener_with_auth_rate_limit_and_timeout(
    db: &mut Trail,
    listener: TcpListener,
    max_requests: Option<usize>,
    auth: ServerAuth,
    rate_limit: ServerRateLimit,
    connection_timeout: Duration,
) -> Result<()> {
    if connection_timeout.is_zero() {
        return Err(Error::InvalidInput(
            "connection timeout must be greater than zero".to_string(),
        ));
    }
    let mut handled = 0usize;
    let mut rate_limiter = HttpRateLimiter::new(rate_limit);
    loop {
        if max_requests.is_some_and(|max| handled >= max) {
            break;
        }
        let (stream, peer_addr) = listener.accept()?;
        let _ = handle_connection(
            db,
            stream,
            peer_addr,
            &auth,
            &mut rate_limiter,
            connection_timeout,
        );
        handled += 1;
    }
    Ok(())
}

pub fn handle_http_request(db: &mut Trail, raw: &[u8]) -> HttpResponse {
    handle_http_request_with_auth(db, raw, &ServerAuth::disabled())
}

pub fn handle_http_request_with_auth(
    db: &mut Trail,
    raw: &[u8],
    auth: &ServerAuth,
) -> HttpResponse {
    match parse_request(raw) {
        Ok(request) => route::route_request(db, request, auth),
        Err(err) => route::error_response(&err),
    }
}

fn handle_connection(
    db: &mut Trail,
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    auth: &ServerAuth,
    rate_limiter: &mut HttpRateLimiter,
    connection_timeout: Duration,
) -> Result<()> {
    stream.set_read_timeout(Some(connection_timeout))?;
    stream.set_write_timeout(Some(connection_timeout))?;
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err(err) => {
            let response = request_read_error_response(&err, connection_timeout);
            let _ = stream
                .write_all(&response.to_http_bytes())
                .and_then(|_| stream.flush());
            return Ok(());
        }
    };
    if let Some(retry_after_secs) = rate_limiter.check(peer_addr.ip()) {
        let response = rate_limited_response(retry_after_secs);
        stream.write_all(&response.to_http_bytes())?;
        stream.flush()?;
        return Ok(());
    }
    let response = route::route_request(db, request, auth);
    stream.write_all(&response.to_http_bytes())?;
    stream.flush()?;
    Ok(())
}

struct HttpRateLimiter {
    config: ServerRateLimit,
    peers: BTreeMap<IpAddr, RateWindow>,
}

struct RateWindow {
    started_at: Instant,
    count: usize,
}

impl HttpRateLimiter {
    fn new(config: ServerRateLimit) -> Self {
        Self {
            config,
            peers: BTreeMap::new(),
        }
    }

    fn check(&mut self, peer: IpAddr) -> Option<u64> {
        let max_requests = self.config.max_requests?;
        let now = Instant::now();
        let window = self.config.window;
        let entry = self.peers.entry(peer).or_insert(RateWindow {
            started_at: now,
            count: 0,
        });
        if now.duration_since(entry.started_at) >= window {
            entry.started_at = now;
            entry.count = 0;
        }
        if entry.count >= max_requests {
            let elapsed = now.duration_since(entry.started_at);
            return Some(window.saturating_sub(elapsed).as_secs().max(1));
        }
        entry.count += 1;
        None
    }
}

fn rate_limited_response(retry_after_secs: u64) -> HttpResponse {
    let body = serde_json::to_vec(&json!({
        "error": {
            "message": format!("rate limit exceeded; retry after {retry_after_secs} seconds"),
            "code": 2,
            "retry_after_secs": retry_after_secs
        }
    }))
    .unwrap_or_else(|_| b"{\"error\":{\"message\":\"rate limit exceeded\",\"code\":2}}".to_vec());
    HttpResponse {
        status: 429,
        reason: "Too Many Requests",
        extra_headers: vec![("Retry-After", retry_after_secs.to_string())],
        body,
    }
}

fn request_read_error_response(err: &Error, timeout: Duration) -> HttpResponse {
    if is_timeout_error(err) {
        let body = serde_json::to_vec(&json!({
            "error": {
                "message": format!("HTTP request timed out after {}", format_duration(timeout)),
                "code": 2
            }
        }))
        .unwrap_or_else(|_| {
            b"{\"error\":{\"message\":\"HTTP request timed out\",\"code\":2}}".to_vec()
        });
        return HttpResponse {
            status: 408,
            reason: "Request Timeout",
            extra_headers: Vec::new(),
            body,
        };
    }
    route::error_response(err)
}

fn is_timeout_error(err: &Error) -> bool {
    matches!(
        err,
        Error::Io(io_err)
            if matches!(io_err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
    )
}

fn format_duration(duration: Duration) -> String {
    if duration.as_secs() == 0 {
        format!("{} milliseconds", duration.as_millis())
    } else {
        format!("{} seconds", duration.as_secs())
    }
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut bytes_read = 0usize;
    let first_line = read_http_line_limited(&mut reader, &mut bytes_read)?;
    if first_line.trim().is_empty() {
        return Err(Error::InvalidInput("empty HTTP request".to_string()));
    }
    let mut content_length = 0usize;
    let mut headers = BTreeMap::new();
    loop {
        let line = read_http_line_limited(&mut reader, &mut bytes_read)?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(length) = parse_http_header_line(&mut headers, trimmed)? {
            content_length = length;
            if content_length > MAX_HTTP_REQUEST_BYTES {
                return Err(Error::InvalidInput(format!(
                    "HTTP request body is {content_length} bytes, exceeding limit {MAX_HTTP_REQUEST_BYTES}"
                )));
            }
            add_http_request_bytes(bytes_read, content_length)?;
        }
    }
    add_http_request_bytes(bytes_read, content_length)?;
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    parse_request_parts(&first_line, headers, body)
}

fn read_http_line_limited<R: Read>(reader: &mut R, bytes_read: &mut usize) -> Result<String> {
    let mut line = Vec::new();
    let mut terminated = false;
    loop {
        let mut byte = [0_u8; 1];
        let read = reader.read(&mut byte)?;
        if read == 0 {
            break;
        }
        *bytes_read = add_http_request_bytes(*bytes_read, read)?;
        line.push(byte[0]);
        if byte[0] == b'\n' {
            terminated = true;
            break;
        }
    }
    if !terminated {
        return Err(Error::InvalidInput(
            "incomplete HTTP request head; missing CRLF terminator".to_string(),
        ));
    }
    if !line.ends_with(b"\r\n") {
        return Err(Error::InvalidInput(
            "HTTP request head lines must end with CRLF".to_string(),
        ));
    }
    String::from_utf8(line)
        .map_err(|_| Error::InvalidInput("HTTP request head must be valid UTF-8".to_string()))
}

fn parse_request(raw: &[u8]) -> Result<HttpRequest> {
    if raw.len() > MAX_HTTP_REQUEST_BYTES {
        return Err(http_request_size_error(raw.len()));
    }
    let Some(separator_idx) = find_header_body_separator(raw) else {
        return Err(Error::InvalidInput("malformed HTTP request".to_string()));
    };
    validate_raw_http_head_crlf(&raw[..separator_idx])?;
    let head = std::str::from_utf8(&raw[..separator_idx])
        .map_err(|_| Error::InvalidInput("HTTP request head must be valid UTF-8".to_string()))?;
    let body = &raw[separator_idx + 4..];
    if head.is_empty() {
        return Err(Error::InvalidInput("empty HTTP request".to_string()));
    }
    let mut lines = head.split("\r\n");
    let first_line = lines
        .next()
        .ok_or_else(|| Error::InvalidInput("empty HTTP request".to_string()))?;
    let mut headers = BTreeMap::new();
    let mut declared_content_length = None;
    for line in lines {
        if let Some(length) = parse_http_header_line(&mut headers, line)? {
            declared_content_length = Some(length);
        }
    }
    if let Some(content_length) = declared_content_length {
        let body_len = body.len();
        if content_length != body_len {
            return Err(Error::InvalidInput(format!(
                "Content-Length declared {content_length} bytes but request body has {body_len} bytes"
            )));
        }
    } else if !body.is_empty() {
        return Err(Error::InvalidInput(
            "HTTP request body requires Content-Length".to_string(),
        ));
    }
    parse_request_parts(first_line, headers, body.to_vec())
}

fn find_header_body_separator(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

fn validate_raw_http_head_crlf(head: &[u8]) -> Result<()> {
    for (idx, byte) in head.iter().enumerate() {
        match byte {
            b'\n' if idx == 0 || head[idx - 1] != b'\r' => {
                return Err(Error::InvalidInput(
                    "HTTP request head lines must end with CRLF".to_string(),
                ));
            }
            b'\r' if head.get(idx + 1) != Some(&b'\n') => {
                return Err(Error::InvalidInput(
                    "HTTP request head lines must end with CRLF".to_string(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_http_header_line(
    headers: &mut BTreeMap<String, String>,
    line: &str,
) -> Result<Option<usize>> {
    let Some((name, value)) = line.split_once(':') else {
        return Err(Error::InvalidInput(
            "malformed HTTP header line".to_string(),
        ));
    };
    if name.is_empty() || name != name.trim() || !name.chars().all(is_http_header_name_char) {
        return Err(Error::InvalidInput(
            "malformed HTTP header name".to_string(),
        ));
    }
    let value = value.trim();
    if value.chars().any(|ch| ch.is_control() && ch != '\t') {
        return Err(Error::InvalidInput(format!(
            "invalid control character in HTTP header `{name}`"
        )));
    }
    let key = name.to_ascii_lowercase();
    if key == "transfer-encoding" {
        return Err(Error::InvalidInput(
            "Transfer-Encoding is not supported by Trail HTTP".to_string(),
        ));
    }
    if duplicate_sensitive_header(&key) && headers.contains_key(&key) {
        return Err(Error::InvalidInput(format!(
            "duplicate HTTP header `{name}`"
        )));
    }
    if key == "content-length" {
        if headers.contains_key(&key) {
            return Err(Error::InvalidInput(
                "duplicate Content-Length header".to_string(),
            ));
        }
        let content_length = parse_content_length_header(value)?;
        headers.insert(key, value.to_string());
        return Ok(Some(content_length));
    }
    headers.insert(key, value.to_string());
    Ok(None)
}

fn parse_content_length_header(value: &str) -> Result<usize> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::InvalidInput(
            "invalid Content-Length header".to_string(),
        ));
    }
    value
        .parse()
        .map_err(|_| Error::InvalidInput("invalid Content-Length header".to_string()))
}

fn duplicate_sensitive_header(key: &str) -> bool {
    matches!(
        key,
        "authorization" | "x-trail-token" | "origin" | "idempotency-key" | "host"
    )
}

fn is_http_header_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '.'
                | '^'
                | '_'
                | '`'
                | '|'
                | '~'
        )
}

fn add_http_request_bytes(current: usize, added: usize) -> Result<usize> {
    let total = current.checked_add(added).unwrap_or(usize::MAX);
    if total > MAX_HTTP_REQUEST_BYTES {
        return Err(http_request_size_error(total));
    }
    Ok(total)
}

fn http_request_size_error(bytes: usize) -> Error {
    Error::InvalidInput(format!(
        "HTTP request is {bytes} bytes, exceeding limit {MAX_HTTP_REQUEST_BYTES}"
    ))
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
    let version = parts
        .next()
        .ok_or_else(|| Error::InvalidInput("missing HTTP version".to_string()))?;
    if parts.next().is_some() {
        return Err(Error::InvalidInput(
            "malformed HTTP request line".to_string(),
        ));
    }
    if !method.chars().all(is_http_header_name_char) {
        return Err(Error::InvalidInput("malformed HTTP method".to_string()));
    }
    if !path.starts_with('/') {
        return Err(Error::InvalidInput(
            "HTTP path must use origin-form starting with `/`".to_string(),
        ));
    }
    validate_http_origin_form_path(path)?;
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(Error::InvalidInput(format!(
            "unsupported HTTP version `{version}`"
        )));
    }
    Ok(HttpRequest {
        method: method.to_string(),
        path: path.to_string(),
        headers,
        body,
    })
}

fn validate_http_origin_form_path(path: &str) -> Result<()> {
    if path.chars().any(char::is_control) {
        return Err(Error::InvalidInput(
            "HTTP path cannot contain control characters".to_string(),
        ));
    }
    if path.contains('\\') {
        return Err(Error::InvalidInput(
            "HTTP path cannot contain backslash separators".to_string(),
        ));
    }
    if path.contains('#') {
        return Err(Error::InvalidInput(
            "HTTP path must not include a fragment".to_string(),
        ));
    }
    Ok(())
}

impl HttpResponse {
    pub fn body_json<T: for<'de> Deserialize<'de>>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(Error::from)
    }

    pub(crate) fn to_http_bytes(&self) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            self.status,
            self.reason,
            self.body.len()
        )
        .into_bytes();
        for (name, value) in &self.extra_headers {
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(value.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"Connection: close\r\n\r\n");
        out.extend_from_slice(&self.body);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::thread;

    #[test]
    fn socket_line_reader_rejects_oversized_line_without_newline() {
        let mut reader = Cursor::new(vec![b'x'; MAX_HTTP_REQUEST_BYTES + 1]);
        let mut bytes_read = 0usize;

        let err = read_http_line_limited(&mut reader, &mut bytes_read).unwrap_err();

        assert!(
            matches!(err, Error::InvalidInput(message) if message.contains("HTTP request is") && message.contains("exceeding limit"))
        );
        assert_eq!(bytes_read, MAX_HTTP_REQUEST_BYTES);
    }

    #[test]
    fn socket_line_reader_counts_prior_request_bytes() {
        let mut reader = Cursor::new(b"overflow\n".to_vec());
        let mut bytes_read = MAX_HTTP_REQUEST_BYTES - 4;

        let err = read_http_line_limited(&mut reader, &mut bytes_read).unwrap_err();

        assert!(
            matches!(err, Error::InvalidInput(message) if message.contains("HTTP request is") && message.contains("exceeding limit"))
        );
        assert_eq!(bytes_read, MAX_HTTP_REQUEST_BYTES);
    }

    #[test]
    fn socket_request_limit_counts_headers_plus_declared_body() {
        let request = format!(
            "POST /v1/health HTTP/1.1\r\nHost: localhost\r\nContent-Length: {MAX_HTTP_REQUEST_BYTES}\r\n\r\n"
        );
        let err = read_request_error_for(request);

        assert!(
            matches!(err, Error::InvalidInput(message) if message.contains("HTTP request is") && message.contains("exceeding limit"))
        );
    }

    #[test]
    fn socket_request_limit_rechecks_headers_after_content_length() {
        let content_length = MAX_HTTP_REQUEST_BYTES - 128;
        let request = format!(
            "POST /v1/health HTTP/1.1\r\nContent-Length: {content_length}\r\nX-Pad: {}\r\n\r\n",
            "x".repeat(256)
        );
        let err = read_request_error_for(request);

        assert!(
            matches!(err, Error::InvalidInput(message) if message.contains("HTTP request is") && message.contains("exceeding limit"))
        );
    }

    #[test]
    fn socket_rejects_duplicate_content_length() {
        let request = "POST /v1/health HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n"
            .to_string();
        let err = read_request_error_for(request);

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("duplicate Content-Length")
        ));
    }

    #[test]
    fn parser_rejects_whitespace_around_header_names() {
        for header_line in [" Content-Length: 0", "Content-Length : 0"] {
            let request = format!("POST /v1/health HTTP/1.1\r\n{header_line}\r\n\r\n");
            let err = parse_request(request.as_bytes()).unwrap_err();
            let message = err.to_string();

            assert!(
                message.contains("malformed HTTP header name"),
                "{header_line} was not rejected as malformed: {message}"
            );
        }
    }

    #[test]
    fn parser_rejects_non_decimal_content_length_values() {
        for value in ["+0", "0x0", "1_000", "5;chunked"] {
            let request = format!("POST /v1/health HTTP/1.1\r\nContent-Length: {value}\r\n\r\n");
            let err = parse_request(request.as_bytes()).unwrap_err();
            let message = err.to_string();

            assert!(
                message.contains("invalid Content-Length header"),
                "{value} was not rejected as invalid: {message}"
            );
        }
    }

    #[test]
    fn parser_rejects_duplicate_security_sensitive_headers() {
        for header in [
            "Authorization",
            "X-Trail-Token",
            "Origin",
            "Idempotency-Key",
            "Host",
        ] {
            let request = format!(
                "POST /v1/health HTTP/1.1\r\n{header}: one\r\n{}: two\r\nContent-Length: 0\r\n\r\n",
                header.to_ascii_lowercase()
            );
            let err = parse_request(request.as_bytes()).unwrap_err();
            let message = err.to_string();

            assert!(
                message.contains("duplicate HTTP header"),
                "{header} was not rejected as duplicate: {message}"
            );
        }
    }

    #[test]
    fn socket_rejects_malformed_header_lines() {
        let request = "GET /v1/health HTTP/1.1\r\nHost localhost\r\n\r\n".to_string();
        let err = read_request_error_for(request);

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("malformed HTTP header line")
        ));
    }

    #[test]
    fn socket_rejects_incomplete_request_line_without_crlf() {
        let request = "GET /v1/health HTTP/1.1".to_string();
        let err = read_request_error_for(request);

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("missing CRLF terminator")
        ));
    }

    #[test]
    fn socket_rejects_lf_only_request_line() {
        let request = "GET /v1/health HTTP/1.1\n\n".to_string();
        let err = read_request_error_for(request);

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("must end with CRLF")
        ));
    }

    #[test]
    fn socket_rejects_header_block_without_blank_line() {
        let request = "GET /v1/health HTTP/1.1\r\nHost: localhost\r\n".to_string();
        let err = read_request_error_for(request);

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("missing CRLF terminator")
        ));
    }

    #[test]
    fn socket_rejects_unsupported_transfer_encoding() {
        let request =
            "POST /v1/health HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".to_string();
        let err = read_request_error_for(request);

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("Transfer-Encoding is not supported")
        ));
    }

    #[test]
    fn raw_parser_rejects_content_length_mismatch() {
        let err =
            parse_request(b"POST /v1/health HTTP/1.1\r\nContent-Length: 5\r\n\r\nabc").unwrap_err();

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("Content-Length declared 5 bytes")
        ));
    }

    #[test]
    fn raw_parser_rejects_body_without_content_length() {
        let err = parse_request(b"POST /v1/health HTTP/1.1\r\n\r\n{}").unwrap_err();

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("body requires Content-Length")
        ));
    }

    #[test]
    fn raw_parser_preserves_non_utf8_body_bytes() {
        let request = b"POST /v1/health HTTP/1.1\r\nContent-Length: 3\r\n\r\n\xff\0a";
        let parsed = parse_request(request).unwrap();

        assert_eq!(parsed.body, b"\xff\0a");
    }

    #[test]
    fn raw_parser_rejects_non_utf8_request_head() {
        let err = parse_request(b"GET /v1/health HTTP/1.1\r\nX-Bad: \xff\r\n\r\n").unwrap_err();

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("request head must be valid UTF-8")
        ));
    }

    #[test]
    fn raw_parser_rejects_bare_lf_or_cr_in_request_head() {
        for request in [
            b"GET /v1/health HTTP/1.1\nHost: localhost\r\n\r\n".as_slice(),
            b"GET /v1/health HTTP/1.1\rHost: localhost\r\n\r\n".as_slice(),
            b"GET /v1/health HTTP/1.1\r\nHost: localhost\nX-Test: yes\r\n\r\n".as_slice(),
        ] {
            let err = parse_request(request).unwrap_err();

            assert!(matches!(
                err,
                Error::InvalidInput(message) if message.contains("must end with CRLF")
            ));
        }
    }

    #[test]
    fn raw_parser_rejects_malformed_request_line() {
        let err = parse_request(b"GET /v1/health HTTP/1.1 extra\r\n\r\n").unwrap_err();

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("malformed HTTP request line")
        ));
    }

    #[test]
    fn raw_parser_rejects_unsupported_http_version() {
        let err = parse_request(b"GET /v1/health HTTP/2.0\r\n\r\n").unwrap_err();

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("unsupported HTTP version")
        ));
    }

    #[test]
    fn raw_parser_rejects_non_origin_form_path() {
        let err = parse_request(b"GET http://127.0.0.1/v1/health HTTP/1.1\r\n\r\n").unwrap_err();

        assert!(matches!(
            err,
            Error::InvalidInput(message) if message.contains("origin-form")
        ));
    }

    #[test]
    fn raw_parser_rejects_malformed_origin_form_targets() {
        for (target, expected) in [
            ("/v1/health#fragment", "fragment"),
            ("/v1\\health", "backslash"),
            ("/v1/health\x01", "control characters"),
        ] {
            let request = format!("GET {target} HTTP/1.1\r\n\r\n");
            let err = parse_request(request.as_bytes()).unwrap_err();
            let message = err.to_string();

            assert!(
                message.contains(expected),
                "{target} was not rejected as {expected}: {message}"
            );
        }
    }

    fn read_request_error_for(request: String) -> Error {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.write_all(request.as_bytes()).unwrap();
        });
        let (mut stream, _) = listener.accept().unwrap();
        let err = read_request(&mut stream).unwrap_err();
        handle.join().unwrap();
        err
    }
}
