use clap::error::ErrorKind as ClapErrorKind;

use super::*;

pub(super) fn args_request_json_errors<I>(args: I) -> bool
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut expect_format = false;
    for arg in args {
        let arg = arg.to_string_lossy();
        if expect_format {
            if arg == "json" {
                return true;
            }
            expect_format = false;
            continue;
        }
        if arg == "--json" || arg == "--format=json" {
            return true;
        }
        if arg == "--format" {
            expect_format = true;
        }
    }
    false
}

pub(super) fn env_requests_json_errors() -> bool {
    std::env::var("TRAIL_FORMAT")
        .map(|value| value.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

pub(super) fn handle_cli_parse_error(err: clap::Error, json: bool) -> ! {
    match err.kind() {
        ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => err.exit(),
        _ if json => {
            let exit_code = err.exit_code();
            render_cli_parse_error(&err, exit_code);
            std::process::exit(exit_code);
        }
        _ => err.exit(),
    }
}

fn render_cli_parse_error(err: &clap::Error, exit_code: i32) {
    let message = err.to_string();
    let value = serde_json::json!({
        "error": {
            "code": "INVALID_INPUT",
            "message": message.trim(),
            "exit_code": exit_code
        }
    });
    eprintln!(
        "{}",
        serde_json::to_string(&value).unwrap_or_else(|_| {
            r#"{"error":{"code":"INVALID_INPUT","message":"invalid CLI input","exit_code":2}}"#
                .to_string()
        })
    );
}

pub(super) fn render_error(err: &Error, json: bool) {
    if json {
        let value = serde_json::json!({
            "error": {
                "code": err.code(),
                "message": err.to_string(),
                "exit_code": err.exit_code()
            }
        });
        eprintln!(
            "{}",
            serde_json::to_string(&value)
                .unwrap_or_else(|_| format!(r#"{{"error":{{"message":"{err}"}}}}"#))
        );
    } else {
        eprintln!("trail: {err}");
    }
}
