use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use super::protocol::{Direction, Frame};
use super::AcpRelayOptions;
use crate::{Error, Result};

pub(crate) const ACP_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const ACP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const ACP_PUMP_DRAIN_TIMEOUT: Duration = ACP_SHUTDOWN_TIMEOUT;
const ACP_CAPTURE_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RelayFinishReason {
    EditorEof,
    EditorError(String),
    AgentEof,
    AgentError(String),
}

pub(crate) trait FrameObserver: Send + Sync + 'static {
    fn observe(&self, frame: &mut Frame) -> Result<()>;
    fn finish(&self, reason: RelayFinishReason);
    fn flush(&self, _timeout: Duration) -> bool {
        true
    }
}

pub(crate) struct StdioRelay<O: FrameObserver> {
    observer: Arc<O>,
}

impl<O: FrameObserver> StdioRelay<O> {
    pub(crate) fn new(observer: Arc<O>) -> Self {
        Self { observer }
    }

    pub(crate) fn run(self, options: &AcpRelayOptions) -> Result<()> {
        if options.upstream_command.is_empty() {
            return Err(Error::InvalidInput(
                "ACP relay requires an upstream command after `--`".to_string(),
            ));
        }

        let (upstream_program, upstream_args) = super::confined_acp_command(
            &options.upstream_command,
            &options.workspace_root,
            &options.db_dir,
            options.materialize,
        )?;
        let mut command = Command::new(upstream_program);
        command
            .args(upstream_args)
            .envs(&options.upstream_env)
            .current_dir(&options.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|err| {
            Error::InvalidInput(format!(
                "failed to launch upstream ACP agent `{}`: {err}",
                options.upstream_command[0]
            ))
        })?;

        let child_stdin = child.stdin.take().ok_or_else(|| {
            Error::InvalidInput("failed to open upstream ACP stdin pipe".to_string())
        })?;
        let child_stdout = child.stdout.take().ok_or_else(|| {
            Error::InvalidInput("failed to open upstream ACP stdout pipe".to_string())
        })?;
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                let _ = copy_upstream_stderr(stderr);
            });
        }

        let (done_tx, done_rx) = mpsc::channel();
        let stop_editor = Arc::new(AtomicBool::new(false));
        let editor_observer = Arc::clone(&self.observer);
        let editor_done = done_tx.clone();
        let editor_stop = Arc::clone(&stop_editor);
        let editor_handle = thread::spawn(move || {
            let result = pump_editor(child_stdin, editor_observer, editor_stop);
            let _ = editor_done.send(PumpDone::Editor(result));
        });

        let agent_observer = Arc::clone(&self.observer);
        let agent_handle = thread::spawn(move || {
            let result = pump(
                BufReader::new(child_stdout),
                io::stdout(),
                Direction::AgentToClient,
                agent_observer,
            );
            let _ = done_tx.send(PumpDone::Agent(result));
        });

        let first = done_rx.recv().map_err(|err| {
            Error::InvalidInput(format!("ACP relay pump failed before startup: {err}"))
        })?;
        let reason = finish_reason(&first);
        if matches!(first, PumpDone::Agent(_)) {
            stop_editor.store(true, Ordering::Release);
        }
        let exit = wait_bounded(&mut child)?;
        let second = done_rx.recv_timeout(ACP_PUMP_DRAIN_TIMEOUT).map_err(|_| {
            Error::InvalidInput(
                "ACP relay pump did not stop before the finalization boundary".to_string(),
            )
        })?;
        editor_handle.join().map_err(|_| {
            Error::InvalidInput("ACP editor pump panicked during shutdown".to_string())
        })?;
        agent_handle.join().map_err(|_| {
            Error::InvalidInput("ACP agent pump panicked during shutdown".to_string())
        })?;

        let (editor_result, agent_result) = pump_results(first, second)?;
        self.observer.finish(reason);
        self.observer.flush(ACP_CAPTURE_FLUSH_TIMEOUT);
        editor_result.map_err(Error::Io)?;
        agent_result.map_err(Error::Io)?;
        if exit.timed_out {
            Ok(())
        } else {
            ensure_success(exit.status)
        }
    }
}

enum PumpDone {
    Editor(io::Result<()>),
    Agent(io::Result<()>),
}

fn finish_reason(done: &PumpDone) -> RelayFinishReason {
    match done {
        PumpDone::Editor(Ok(())) => RelayFinishReason::EditorEof,
        PumpDone::Editor(Err(err)) => RelayFinishReason::EditorError(err.to_string()),
        PumpDone::Agent(Ok(())) => RelayFinishReason::AgentEof,
        PumpDone::Agent(Err(err)) => RelayFinishReason::AgentError(err.to_string()),
    }
}

fn pump_results(first: PumpDone, second: PumpDone) -> Result<(io::Result<()>, io::Result<()>)> {
    match (first, second) {
        (PumpDone::Editor(editor), PumpDone::Agent(agent))
        | (PumpDone::Agent(agent), PumpDone::Editor(editor)) => Ok((editor, agent)),
        _ => Err(Error::InvalidInput(
            "ACP relay received duplicate pump completion".to_string(),
        )),
    }
}

#[cfg(unix)]
fn pump_editor<W, O>(mut writer: W, observer: Arc<O>, stop: Arc<AtomicBool>) -> io::Result<()>
where
    W: Write,
    O: FrameObserver,
{
    let mut pending = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        if stop.load(Ordering::Acquire) {
            return writer.flush();
        }
        let mut descriptor = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 25) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if ready == 0 || stop.load(Ordering::Acquire) {
            continue;
        }
        let read =
            unsafe { libc::read(libc::STDIN_FILENO, chunk.as_mut_ptr().cast(), chunk.len()) };
        if read < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if read == 0 {
            if !pending.is_empty() {
                forward_raw_frame(
                    std::mem::take(&mut pending),
                    Direction::ClientToAgent,
                    &observer,
                    &mut writer,
                )?;
            }
            return writer.flush();
        }
        pending.extend_from_slice(&chunk[..usize::try_from(read).unwrap_or(0)]);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let remainder = pending.split_off(end + 1);
            let raw = std::mem::replace(&mut pending, remainder);
            forward_raw_frame(raw, Direction::ClientToAgent, &observer, &mut writer)?;
        }
        if pending.len() > ACP_MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ACP frame exceeds the {ACP_MAX_FRAME_BYTES}-byte transport limit"),
            ));
        }
    }
}

#[cfg(not(unix))]
fn pump_editor<W, O>(writer: W, observer: Arc<O>, _stop: Arc<AtomicBool>) -> io::Result<()>
where
    W: Write,
    O: FrameObserver,
{
    pump(
        io::stdin().lock(),
        writer,
        Direction::ClientToAgent,
        observer,
    )
}

fn forward_raw_frame<W, O>(
    raw: Vec<u8>,
    direction: Direction,
    observer: &Arc<O>,
    writer: &mut W,
) -> io::Result<()>
where
    W: Write,
    O: FrameObserver,
{
    if raw.len() > ACP_MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("ACP frame exceeds the {ACP_MAX_FRAME_BYTES}-byte transport limit"),
        ));
    }
    if raw.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(());
    }
    let mut frame = Frame::parse(direction, raw)?;
    observer.observe(&mut frame).map_err(io::Error::other)?;
    writer.write_all(frame.forward_bytes())?;
    writer.flush()
}

fn pump<R, W, O>(
    mut reader: R,
    mut writer: W,
    direction: Direction,
    observer: Arc<O>,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
    O: FrameObserver,
{
    while let Some(mut frame) = read_frame(&mut reader, direction)? {
        observer.observe(&mut frame).map_err(io::Error::other)?;
        writer.write_all(frame.forward_bytes())?;
        writer.flush()?;
    }
    writer.flush()
}

fn read_frame<R: BufRead>(reader: &mut R, direction: Direction) -> io::Result<Option<Frame>> {
    loop {
        let mut raw = Vec::new();
        let bytes = reader
            .take((ACP_MAX_FRAME_BYTES + 1) as u64)
            .read_until(b'\n', &mut raw)?;
        if raw.len() > ACP_MAX_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ACP frame exceeds the {ACP_MAX_FRAME_BYTES}-byte transport limit"),
            ));
        }
        if bytes == 0 {
            return Ok(None);
        }
        if raw.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        return Frame::parse(direction, raw).map(Some);
    }
}

fn copy_upstream_stderr<R: Read>(reader: R) -> io::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut buf = [0u8; 8192];
    loop {
        let bytes = reader.read(&mut buf)?;
        if bytes == 0 {
            return Ok(());
        }
        let mut stderr = io::stderr().lock();
        stderr.write_all(&buf[..bytes])?;
        stderr.flush()?;
    }
}

struct ChildExit {
    status: ExitStatus,
    timed_out: bool,
}

fn wait_bounded(child: &mut Child) -> Result<ChildExit> {
    let deadline = Instant::now() + ACP_SHUTDOWN_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().map_err(Error::Io)? {
            return Ok(ChildExit {
                status,
                timed_out: false,
            });
        }
        if Instant::now() >= deadline {
            terminate_child_tree(child)?;
            return child
                .wait()
                .map(|status| ChildExit {
                    status,
                    timed_out: true,
                })
                .map_err(Error::Io);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn terminate_child_tree(child: &mut Child) -> Result<()> {
    #[cfg(unix)]
    {
        let process_group = i32::try_from(child.id()).map_err(|_| {
            Error::InvalidInput("upstream ACP process id does not fit i32".to_string())
        })?;
        let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(Error::Io(error))
    }
    #[cfg(not(unix))]
    {
        child.kill().map_err(Error::Io)
    }
}

fn ensure_success(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "upstream ACP agent exited with status {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingObserver {
        directions: Mutex<Vec<Direction>>,
    }

    impl FrameObserver for RecordingObserver {
        fn observe(&self, frame: &mut Frame) -> Result<()> {
            self.directions.lock().unwrap().push(frame.direction());
            Ok(())
        }

        fn finish(&self, _reason: RelayFinishReason) {}
    }

    #[test]
    fn pump_preserves_frames_and_skips_blank_lines() {
        let input = b"\n  \r\n {\"method\":\"ext/test\",\"jsonrpc\":\"2.0\"} \r\n";
        let mut output = Vec::new();
        let observer = Arc::new(RecordingObserver::default());
        pump(
            BufReader::new(input.as_slice()),
            &mut output,
            Direction::ClientToAgent,
            Arc::clone(&observer),
        )
        .unwrap();
        assert_eq!(output, &input[b"\n  \r\n".len()..]);
        assert_eq!(
            *observer.directions.lock().unwrap(),
            vec![Direction::ClientToAgent]
        );
    }

    #[test]
    fn frame_limit_accepts_the_boundary_and_rejects_one_byte_above_it() {
        let prefix = br#"{"jsonrpc":"2.0","method":"ext/limit","params":{"data":""#;
        let suffix = b"\"}}\n";
        let payload_len = ACP_MAX_FRAME_BYTES - prefix.len() - suffix.len();
        let mut boundary = Vec::with_capacity(ACP_MAX_FRAME_BYTES);
        boundary.extend_from_slice(prefix);
        boundary.resize(boundary.len() + payload_len, b'x');
        boundary.extend_from_slice(suffix);
        assert_eq!(boundary.len(), ACP_MAX_FRAME_BYTES);

        let mut reader = io::Cursor::new(boundary.clone());
        let frame = read_frame(&mut reader, Direction::ClientToAgent)
            .unwrap()
            .unwrap();
        assert_eq!(frame.raw_bytes().len(), ACP_MAX_FRAME_BYTES);

        boundary.insert(boundary.len() - suffix.len(), b'x');
        let mut reader = io::Cursor::new(boundary);
        let error = match read_frame(&mut reader, Direction::ClientToAgent) {
            Err(error) => error,
            Ok(_) => panic!("oversized frame was accepted"),
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("transport limit"));
    }
}
