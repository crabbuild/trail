use super::*;

pub(crate) fn run_command_with_timeout(
    command: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Result<CommandRunResult> {
    let started = Instant::now();
    let mut child = match Command::new(&command[0])
        .args(&command[1..])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return Ok(CommandRunResult {
                success: false,
                exit_code: None,
                timed_out: false,
                duration_ms: elapsed_ms(started.elapsed()),
                stdout: Vec::new(),
                stderr: err.to_string().into_bytes(),
            });
        }
    };

    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            return Ok(CommandRunResult {
                success: output.status.success(),
                exit_code: output.status.code(),
                timed_out: false,
                duration_ms: elapsed_ms(started.elapsed()),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let output = child.wait_with_output()?;
            return Ok(CommandRunResult {
                success: false,
                exit_code: output.status.code(),
                timed_out: true,
                duration_ms: elapsed_ms(started.elapsed()),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
