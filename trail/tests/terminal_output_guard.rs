use std::fs;
use std::path::{Path, PathBuf};

/// Command adapters describe terminal semantics. The only low-level terminal
/// writer is `render/ui.rs`, which owns buffering, ANSI, pager, and progress.
#[test]
fn command_adapters_do_not_write_directly_to_the_terminal() {
    let command_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli/command");
    let mut files = Vec::new();
    collect_rust_files(&command_root.join("handler"), &mut files);
    collect_rust_files(&command_root.join("render"), &mut files);
    files.push(command_root.join("handler.rs"));
    files.push(command_root.join("render.rs"));

    for file in files {
        if file.ends_with("render/ui.rs") {
            continue;
        }
        let source = fs::read_to_string(&file).expect("read command source");
        for forbidden in ["println!", "eprintln!", "print!", "\\x1b[", "\\x1B["] {
            assert!(
                !source.contains(forbidden),
                "{} bypasses the terminal renderer with {forbidden}",
                file.display()
            );
        }
    }
}

fn collect_rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read command source directory") {
        let entry = entry.expect("read command source entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}
