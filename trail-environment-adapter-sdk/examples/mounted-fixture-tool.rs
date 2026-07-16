use std::path::Path;

fn main() {
    let mut arguments = std::env::args().skip(1);
    let behavior = arguments.next().unwrap_or_default();
    let path = arguments.next().unwrap_or_default();
    let input = arguments.next();
    if path.is_empty() {
        eprintln!("mounted-fixture-tool requires an output path");
        std::process::exit(2);
    }
    let path = Path::new(&path);
    let current_directory = std::env::current_dir().unwrap_or_default();
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("cannot create fixture output parent: {error}");
            std::process::exit(3);
        }
    }
    let result = match behavior.as_str() {
        "success" => std::env::current_dir()
            .and_then(|directory| {
                let input = input.as_deref().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "success requires an input path",
                    )
                })?;
                let input = std::fs::read_to_string(input)?;
                Ok(format!(
                    "{}|{}\n",
                    directory.to_string_lossy(),
                    input.trim()
                ))
            })
            .and_then(|contents| std::fs::write(path, contents).map(|_| String::new())),
        "fail" => std::fs::write(path, b"partial").map(|_| String::new()),
        "hang" => {
            if let Err(error) = std::fs::write(path, b"partial") {
                Err(error)
            } else {
                let home = std::env::var_os("HOME").unwrap_or_default();
                let ready = Path::new(&home).join("running");
                if let Err(error) = std::fs::write(&ready, std::process::id().to_string()) {
                    Err(error)
                } else {
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            }
        }
        "source_write" => std::fs::write(path, b"leak").map(|_| String::new()),
        "source_read" => {
            let input = input.as_deref().unwrap_or("input.txt");
            std::fs::read_to_string(input)
                .and_then(|contents| std::fs::write(path, contents))
                .map(|_| String::new())
        }
        _ => {
            eprintln!("unknown mounted fixture behavior `{behavior}`");
            std::process::exit(4);
        }
    };
    if let Err(error) = result {
        eprintln!(
            "mounted fixture action failed in `{}` for `{}`: {error}",
            current_directory.display(),
            path.display()
        );
        std::process::exit(5);
    }
    if behavior == "fail" {
        std::process::exit(23);
    }
}
