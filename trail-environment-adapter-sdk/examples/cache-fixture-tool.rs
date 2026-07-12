use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("cache-fixture-tool: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let behavior = env::args().nth(1).unwrap_or_else(|| "populate".to_string());
    let cache = PathBuf::from(env::var_os("TRAIL_FIXTURE_CACHE").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "TRAIL_FIXTURE_CACHE was not injected by the Trail host",
        )
    })?);
    fs::create_dir_all(&cache)?;
    if behavior == "escape" {
        let parent = cache.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "cache namespace has no parent")
        })?;
        fs::write(parent.join("plugin-cache-escape"), b"escaped\n")?;
        return Err(io::Error::other("cache namespace escape unexpectedly succeeded").into());
    }

    let counter_path = cache.join("counter");
    let previous = fs::read_to_string(&counter_path)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let current = previous
        .checked_add(1)
        .ok_or_else(|| io::Error::other("cache counter overflow"))?;
    fs::write(&counter_path, format!("{current}\n"))?;

    let output = PathBuf::from("generated");
    fs::create_dir_all(&output)?;
    let namespace = cache
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "cache namespace is not UTF-8")
        })?;
    fs::write(
        output.join("cache-observation.txt"),
        format!("{namespace}|{current}\n"),
    )?;
    Ok(())
}
