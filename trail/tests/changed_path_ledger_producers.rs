use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use trail::{Error, InitImportMode, PatchDocument, Trail, WorktreeState};

const INVENTORY: &str = include_str!("fixtures/changed_path_producers.v1");
const RAW_MUTATION_INVENTORY: &str = include_str!("fixtures/changed_path_raw_mutations.v1");
const PRODUCER_TAG: &str = "TRAIL_FS_PRODUCER:";

// These are producer-facing filesystem mutation boundaries. Unlike producer
// annotations, calls to these sinks are discovered across the source tree, so
// adding a new caller requires an explicit review instead of merely omitting a
// tag and escaping the inventory comparison.
const DISCOVERED_MUTATION_SINKS: &[&str] = &[
    "remove_visible_files_absent_from_target",
    "materialize_files",
    "prepare_checkout_workdir",
    "materialize_root_files_at_streaming",
    "materialize_lane_workdir_at_paths_with_neighbors",
    "materialize_lane_root_staged",
    "materialize_sparse_lane_workdir_paths",
    "materialize_full_lane_workdir_staged",
    "rescue_replaced_lane_workdir_path",
    "rescue_dirty_lane_workdir",
    "apply_lane_patch_workdir_projection",
    "apply_rewind_workdir_projection",
    "materialize_files_at",
    "sync_lane_workdir",
    "complete_workspace_checkpoint",
    "ensure_upper_file_under_barrier",
    "write_all_file_at",
];

const RAW_MUTATION_SINKS: &[&str] = &[
    "fs::write",
    "fs::rename",
    "fs::remove_file",
    "fs::remove_dir",
    "fs::remove_dir_all",
    "fs::create_dir",
    "fs::create_dir_all",
    "fs::copy",
    "fs::set_permissions",
    "File::create",
    "symlink_file",
];

const OPEN_OPTIONS_MUTATION_SINKS: &[(&str, &str)] = &[
    (".create", "OpenOptions::create"),
    (".write", "OpenOptions::write"),
    (".truncate", "OpenOptions::truncate"),
    (".create_new", "OpenOptions::create_new"),
    (".append", "OpenOptions::append"),
];

// Implementation calls which are subordinate to an inventoried producer and
// are not independently callable command producers. New entries here are a
// deliberate review boundary and must name an exact function and sink.
const REVIEWED_INTERNAL_MUTATION_CALLERS: &[(&str, &str, &str)] = &[
    (
        "db/lane/lifecycle.rs",
        "materialize_lane_workdir_at_paths_with_neighbors",
        "materialize_lane_root_staged",
    ),
    (
        "db/lane/workdir/sync.rs",
        "materialize_full_lane_workdir_staged",
        "materialize_lane_root_staged",
    ),
    (
        "db/lane/workdir/sync.rs",
        "sync_nfs_cow_lane_workdir",
        "rescue_dirty_lane_workdir",
    ),
    (
        "db/lane/rewind.rs",
        "apply_rewind_workdir_projection",
        "materialize_files_at",
    ),
    (
        "db/lane/patching.rs",
        "apply_lane_patch_workdir_projection",
        "materialize_files_at",
    ),
    (
        "db/lane/workdir/sync.rs",
        "materialize_full_lane_workdir_staged",
        "materialize_files_at",
    ),
    (
        "db/lane/workdir/sync.rs",
        "materialize_sparse_lane_workdir_paths",
        "materialize_files_at",
    ),
    (
        "db/storage/content.rs",
        "materialize_files",
        "materialize_files_at",
    ),
    (
        "db/record/checkout.rs",
        "remove_visible_files_absent_from_target",
        "fs::remove_file",
    ),
    (
        "db/lane/lifecycle.rs",
        "write_sparse_workdir_manifest",
        "fs::create_dir_all",
    ),
    (
        "db/lane/workdir/record.rs",
        "run_lane_record_after_c2_write",
        "fs::write",
    ),
    (
        "db/lane/workdir/sync.rs",
        "materialize_full_lane_workdir_staged",
        "fs::remove_file",
    ),
    (
        "db/lane/workdir/sync.rs",
        "materialize_full_lane_workdir_staged",
        "fs::create_dir_all",
    ),
    (
        "db/lane/workdir/sync.rs",
        "materialize_full_lane_workdir_staged",
        "fs::remove_dir_all",
    ),
    (
        "db/lane/workdir/sync.rs",
        "rescue_dirty_lane_workdir",
        "fs::create_dir_all",
    ),
    (
        "db/lane/workdir/sync.rs",
        "rescue_dirty_lane_workdir",
        "fs::write",
    ),
    (
        "db/lane/workdir/sync.rs",
        "rescue_replaced_lane_workdir_path",
        "fs::create_dir_all",
    ),
    (
        "db/lane/workdir/sync.rs",
        "rescue_replaced_lane_workdir_path",
        "fs::write",
    ),
    (
        "db/lane/workdir/sync.rs",
        "create_unique_lane_workdir_rescue_dir",
        "fs::create_dir",
    ),
    (
        "db/lane/workdir/sync.rs",
        "create_unique_lane_workdir_sync_stage_dir",
        "fs::create_dir",
    ),
    (
        "db/lane/workdir/sync.rs",
        "replace_lane_workdir_with_stage",
        "fs::rename",
    ),
    (
        "db/lane/workdir/sync.rs",
        "move_existing_lane_workdir_to_backup",
        "fs::rename",
    ),
    (
        "db/lane/workdir/sync.rs",
        "remove_existing_lane_workdir_path",
        "fs::remove_dir_all",
    ),
    (
        "db/lane/workdir/sync.rs",
        "remove_existing_lane_workdir_path",
        "fs::remove_file",
    ),
    ("db/lane/workdir/sync.rs", "begin", "fs::create_dir_all"),
    ("db/lane/workdir/sync.rs", "begin", "fs::remove_dir_all"),
    ("db/lane/workdir/sync.rs", "commit", "fs::remove_dir_all"),
    ("db/lane/workdir/sync.rs", "rollback", "fs::remove_dir_all"),
    (
        "db/lane/workdir/sync.rs",
        "restore_sparse_hydration_snapshot",
        "fs::create_dir_all",
    ),
    (
        "db/lane/workdir/sync.rs",
        "remove_sparse_hydration_target",
        "fs::remove_dir_all",
    ),
    (
        "db/lane/workdir/sync.rs",
        "remove_sparse_hydration_target",
        "fs::remove_file",
    ),
    (
        "db/lane/workdir/sync.rs",
        "rescue_dirty_lane_workdir",
        "fs::copy",
    ),
    (
        "db/lane/workdir/sync.rs",
        "rescue_replaced_lane_workdir_path",
        "fs::copy",
    ),
    (
        "db/lane/workdir/sync.rs",
        "snapshot_sparse_hydration_target",
        "fs::copy",
    ),
    (
        "db/lane/workdir/sync.rs",
        "restore_sparse_hydration_snapshot",
        "fs::copy",
    ),
    (
        "db/lane/workdir/sync.rs",
        "restore_sparse_hydration_snapshot",
        "fs::set_permissions",
    ),
    (
        "db/lane/workdir/sync.rs",
        "restore_sparse_hydration_snapshot",
        "symlink_file",
    ),
    (
        "db/lane/workdir/sync.rs",
        "sparse_hydration_file_matches_snapshot",
        "fs::set_permissions",
    ),
];

static RUNTIME_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct AuthorityGuard {
    _lock: MutexGuard<'static, ()>,
}

impl AuthorityGuard {
    fn enabled() -> Self {
        let lock = RUNTIME_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        trail::test_support::set_changed_path_authority_override(true);
        Self { _lock: lock }
    }

    fn disabled() -> Self {
        let lock = RUNTIME_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        trail::test_support::set_changed_path_authority_override(false);
        Self { _lock: lock }
    }
}

impl Drop for AuthorityGuard {
    fn drop(&mut self) {
        trail::test_support::set_changed_path_authority_override(false);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ProducerSite {
    status: String,
    source: String,
    function: String,
    site: String,
    producer: String,
    protocol: String,
    sinks: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ReviewedRawMutation {
    class: String,
    source: String,
    function: String,
    sink: String,
    count: usize,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn producer_inventory() -> Vec<ProducerSite> {
    INVENTORY
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| {
            let fields = line.split('|').collect::<Vec<_>>();
            assert_eq!(fields.len(), 7, "invalid producer inventory row: {line}");
            ProducerSite {
                status: fields[0].to_string(),
                source: fields[1].to_string(),
                function: fields[2].to_string(),
                site: fields[3].to_string(),
                producer: fields[4].to_string(),
                protocol: fields[5].to_string(),
                sinks: fields[6].split(',').map(str::to_string).collect(),
            }
        })
        .collect()
}

fn raw_mutation_inventory() -> Vec<ReviewedRawMutation> {
    RAW_MUTATION_INVENTORY
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| {
            let fields = line.split('|').collect::<Vec<_>>();
            assert_eq!(
                fields.len(),
                5,
                "invalid raw mutation inventory row: {line}"
            );
            assert!(
                matches!(fields[0], "reviewed" | "pending_task12"),
                "invalid raw mutation review class in row: {line}"
            );
            ReviewedRawMutation {
                class: fields[0].to_string(),
                source: fields[1].to_string(),
                function: fields[2].to_string(),
                sink: fields[3].to_string(),
                count: fields[4]
                    .parse()
                    .unwrap_or_else(|_| panic!("invalid raw mutation count in row: {line}")),
            }
        })
        .collect()
}

/// Return the exact Rust function body containing `fn name`, ignoring braces
/// in comments, strings, chars, and raw strings. This keeps the source audit
/// tied to a concrete entry point instead of accepting a protocol call placed
/// elsewhere in the file.
fn rust_function_body<'a>(source: &'a str, name: &str) -> &'a str {
    let needle = format!("fn {name}");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("missing inventoried function `{name}`"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("function `{name}` has no body"));
    let bytes = source.as_bytes();
    let mut index = open;
    let mut depth = 0usize;
    let mut state = LexState::Code;
    while index < bytes.len() {
        match state {
            LexState::Code => match bytes[index] {
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    state = LexState::LineComment;
                    index += 2;
                    continue;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    state = LexState::BlockComment(1);
                    index += 2;
                    continue;
                }
                b'"' => state = LexState::String,
                b'\'' => state = LexState::Char,
                b'r' if matches!(bytes.get(index + 1), Some(b'"' | b'#')) => {
                    let mut cursor = index + 1;
                    let mut hashes = 0usize;
                    while bytes.get(cursor) == Some(&b'#') {
                        hashes += 1;
                        cursor += 1;
                    }
                    if bytes.get(cursor) == Some(&b'"') {
                        state = LexState::RawString(hashes);
                        index = cursor;
                    }
                }
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[start..=index];
                    }
                }
                _ => {}
            },
            LexState::LineComment => {
                if bytes[index] == b'\n' {
                    state = LexState::Code;
                }
            }
            LexState::BlockComment(level) => {
                if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    state = LexState::BlockComment(level + 1);
                    index += 2;
                    continue;
                }
                if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    state = if level == 1 {
                        LexState::Code
                    } else {
                        LexState::BlockComment(level - 1)
                    };
                    index += 2;
                    continue;
                }
            }
            LexState::String => {
                if bytes[index] == b'\\' {
                    index += 2;
                    continue;
                }
                if bytes[index] == b'"' {
                    state = LexState::Code;
                }
            }
            LexState::Char => {
                if bytes[index] == b'\\' {
                    index += 2;
                    continue;
                }
                if bytes[index] == b'\'' {
                    state = LexState::Code;
                }
            }
            LexState::RawString(hashes) => {
                if bytes[index] == b'"'
                    && bytes.get(index + 1..index + 1 + hashes)
                        == Some(vec![b'#'; hashes].as_slice())
                {
                    state = LexState::Code;
                    index += hashes;
                }
            }
        }
        index += 1;
    }
    panic!("unterminated inventoried function `{name}`")
}

#[derive(Clone, Copy)]
enum LexState {
    Code,
    LineComment,
    BlockComment(usize),
    String,
    Char,
    RawString(usize),
}

/// Remove comments and literal contents while preserving executable Rust
/// tokens. Protocol and sink checks run against this representation, so a
/// comment or diagnostic string cannot impersonate a reviewed call.
fn rust_code_only(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = vec![b' '; bytes.len()];
    let mut index = 0usize;
    let mut state = LexState::Code;
    while index < bytes.len() {
        match state {
            LexState::Code => match bytes[index] {
                b'/' if bytes.get(index + 1) == Some(&b'/') => {
                    state = LexState::LineComment;
                    index += 2;
                    continue;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    state = LexState::BlockComment(1);
                    index += 2;
                    continue;
                }
                b'"' => state = LexState::String,
                b'\''
                    if bytes.get(index + 2) == Some(&b'\'')
                        || bytes.get(index + 1) == Some(&b'\\') =>
                {
                    state = LexState::Char;
                }
                b'r' if matches!(bytes.get(index + 1), Some(b'"' | b'#')) => {
                    let mut cursor = index + 1;
                    let mut hashes = 0usize;
                    while bytes.get(cursor) == Some(&b'#') {
                        hashes += 1;
                        cursor += 1;
                    }
                    if bytes.get(cursor) == Some(&b'"') {
                        state = LexState::RawString(hashes);
                        index = cursor;
                    } else {
                        output[index] = bytes[index];
                    }
                }
                _ => output[index] = bytes[index],
            },
            LexState::LineComment => {
                if bytes[index] == b'\n' {
                    output[index] = b'\n';
                    state = LexState::Code;
                }
            }
            LexState::BlockComment(level) => {
                if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    state = LexState::BlockComment(level + 1);
                    index += 2;
                    continue;
                }
                if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    state = if level == 1 {
                        LexState::Code
                    } else {
                        LexState::BlockComment(level - 1)
                    };
                    index += 2;
                    continue;
                }
            }
            LexState::String => {
                if bytes[index] == b'\\' {
                    index += 2;
                    continue;
                }
                if bytes[index] == b'"' {
                    state = LexState::Code;
                }
            }
            LexState::Char => {
                if bytes[index] == b'\\' {
                    index += 2;
                    continue;
                }
                if bytes[index] == b'\'' {
                    state = LexState::Code;
                }
            }
            LexState::RawString(hashes) => {
                if bytes[index] == b'"'
                    && bytes.get(index + 1..index + 1 + hashes)
                        == Some(vec![b'#'; hashes].as_slice())
                {
                    state = LexState::Code;
                    index += hashes;
                }
            }
        }
        index += 1;
    }
    String::from_utf8(output).unwrap()
}

fn rust_call_offsets(code: &str, callable: &str) -> Vec<usize> {
    code.match_indices(callable)
        .filter_map(|(start, _)| {
            let before = code[..start].chars().next_back();
            if !callable.starts_with('.')
                && before.is_some_and(|ch| ch == '_' || ch.is_ascii_alphanumeric())
            {
                return None;
            }
            if !code[start + callable.len()..].trim_start().starts_with('(') {
                return None;
            }
            // `fn sink(` declares a function; it is not a concrete call to
            // the mutation boundary.
            if code[..start].trim_end().ends_with("fn") {
                return None;
            }
            Some(start)
        })
        .collect()
}

fn contains_rust_call(code: &str, callable: &str) -> bool {
    !rust_call_offsets(code, callable).is_empty()
}

fn rust_call_argument(code_after_callable: &str) -> Option<&str> {
    let call = code_after_callable.trim_start();
    let call = call.strip_prefix('(')?;
    let mut depth = 1usize;
    for (index, byte) in call.as_bytes().iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&call[..index]);
                }
            }
            _ => {}
        }
    }
    None
}

fn mutating_open_options_call_offsets(code: &str) -> Vec<(usize, &'static str)> {
    let mut calls = Vec::new();
    for start in rust_call_offsets(code, "OpenOptions::new") {
        let tail = &code[start..];
        let statement_end = tail.find(';').unwrap_or(tail.len());
        let statement = &tail[..statement_end];
        let (chain, base) =
            if let Some(open) = rust_call_offsets(statement, ".open").into_iter().next() {
                (&statement[..open], start)
            } else {
                let (_, function_range) = enclosing_rust_function_range(code, start).unwrap();
                let declaration_start = [b';', b'{', b'}']
                    .into_iter()
                    .filter_map(|delimiter| {
                        code.as_bytes()[function_range.start..start]
                            .iter()
                            .rposition(|byte| *byte == delimiter)
                    })
                    .max()
                    .map(|relative| function_range.start + relative + 1)
                    .unwrap_or(function_range.start);
                let declaration = &code[declaration_start..start];
                let Some(equals) = declaration.rfind('=') else {
                    continue;
                };
                let mut binding = declaration[..equals].trim();
                let Some(rest) = binding.strip_prefix("let ") else {
                    continue;
                };
                binding = rest.trim_start();
                if let Some(rest) = binding.strip_prefix("mut ") {
                    binding = rest.trim_start();
                }
                let name_len = binding
                    .chars()
                    .take_while(|ch| *ch == '_' || ch.is_ascii_alphanumeric())
                    .map(char::len_utf8)
                    .sum::<usize>();
                if name_len == 0 {
                    continue;
                }
                let binding = &binding[..name_len];
                let configured = &code[start + statement_end + 1..function_range.end];
                let open_callable = format!("{binding}.open");
                let Some(open) = rust_call_offsets(configured, &open_callable)
                    .into_iter()
                    .next()
                else {
                    continue;
                };
                (&configured[..open], start + statement_end + 1)
            };
        for (callable, sink) in OPEN_OPTIONS_MUTATION_SINKS {
            for relative in rust_call_offsets(chain, callable) {
                let argument = rust_call_argument(&chain[relative + callable.len()..]).unwrap();
                if argument.trim() == "false" {
                    continue;
                }
                calls.push((base + relative, *sink));
            }
        }
    }
    calls
}

fn rust_function_range(source: &str, name: &str) -> std::ops::Range<usize> {
    let body = rust_function_body(source, name);
    let start = body.as_ptr() as usize - source.as_ptr() as usize;
    start..start + body.len()
}

fn enclosing_rust_function_range(
    code: &str,
    offset: usize,
) -> Option<(&str, std::ops::Range<usize>)> {
    code[..offset]
        .match_indices("fn ")
        .map(|(start, _)| start)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .find_map(|start| {
            let before = code[..start].chars().next_back();
            if before.is_some_and(|ch| ch == '_' || ch.is_ascii_alphanumeric()) {
                return None;
            }
            let name_start = start + 3;
            let name_len = code[name_start..]
                .chars()
                .take_while(|ch| *ch == '_' || ch.is_ascii_alphanumeric())
                .map(char::len_utf8)
                .sum::<usize>();
            if name_len == 0 {
                return None;
            }
            let open = code[start..].find('{').map(|relative| start + relative)?;
            let mut depth = 0usize;
            for (relative, byte) in code.as_bytes()[open..].iter().enumerate() {
                match byte {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return (offset <= open + relative).then_some((
                                &code[name_start..name_start + name_len],
                                start..open + relative + 1,
                            ));
                        }
                    }
                    _ => {}
                }
            }
            None
        })
}

fn enclosing_rust_function(code: &str, offset: usize) -> Option<&str> {
    enclosing_rust_function_range(code, offset).map(|(name, _)| name)
}

fn rust_source_files(source_root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![source_root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProductionCfgValue {
    False,
    True,
    Unknown,
}

impl ProductionCfgValue {
    fn not(self) -> Self {
        match self {
            Self::False => Self::True,
            Self::True => Self::False,
            Self::Unknown => Self::Unknown,
        }
    }
}

struct ProductionCfgParser<'a> {
    input: &'a [u8],
    index: usize,
}

impl ProductionCfgParser<'_> {
    fn skip_whitespace(&mut self) {
        while self
            .input
            .get(self.index)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.index += 1;
        }
    }

    fn parse_identifier(&mut self) -> String {
        self.skip_whitespace();
        let start = self.index;
        while self
            .input
            .get(self.index)
            .is_some_and(|byte| *byte == b'_' || byte.is_ascii_alphanumeric())
        {
            self.index += 1;
        }
        std::str::from_utf8(&self.input[start..self.index])
            .unwrap()
            .to_string()
    }

    fn parse_expression(&mut self) -> ProductionCfgValue {
        let identifier = self.parse_identifier();
        self.skip_whitespace();
        if self.input.get(self.index) != Some(&b'(') {
            let bare_predicate = matches!(self.input.get(self.index), None | Some(b',' | b')'));
            while !matches!(self.input.get(self.index), None | Some(b',' | b')')) {
                self.index += 1;
            }
            return if identifier == "test" && bare_predicate {
                ProductionCfgValue::False
            } else {
                ProductionCfgValue::Unknown
            };
        }
        self.index += 1;
        let mut arguments = Vec::new();
        loop {
            self.skip_whitespace();
            if self.input.get(self.index) == Some(&b')') {
                self.index += 1;
                break;
            }
            arguments.push(self.parse_expression());
            self.skip_whitespace();
            match self.input.get(self.index) {
                Some(b',') => self.index += 1,
                Some(b')') => {
                    self.index += 1;
                    break;
                }
                _ => break,
            }
        }
        match identifier.as_str() {
            "not" => arguments
                .into_iter()
                .next()
                .unwrap_or(ProductionCfgValue::Unknown)
                .not(),
            "all" => {
                if arguments.contains(&ProductionCfgValue::False) {
                    ProductionCfgValue::False
                } else if arguments
                    .iter()
                    .all(|value| *value == ProductionCfgValue::True)
                {
                    ProductionCfgValue::True
                } else {
                    ProductionCfgValue::Unknown
                }
            }
            "any" => {
                if arguments.contains(&ProductionCfgValue::True) {
                    ProductionCfgValue::True
                } else if arguments
                    .iter()
                    .all(|value| *value == ProductionCfgValue::False)
                {
                    ProductionCfgValue::False
                } else {
                    ProductionCfgValue::Unknown
                }
            }
            _ => ProductionCfgValue::Unknown,
        }
    }
}

fn cfg_item_is_disabled_in_production(attribute: &str) -> bool {
    let Some(cfg) = attribute.strip_prefix("#[cfg") else {
        return false;
    };
    let cfg = cfg.trim_start();
    let Some(expression) = cfg.strip_prefix('(') else {
        return false;
    };
    ProductionCfgParser {
        input: expression.as_bytes(),
        index: 0,
    }
    .parse_expression()
        == ProductionCfgValue::False
}

fn nonproduction_cfg_ranges(code: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    for (attribute_start, _) in code.match_indices("#[cfg") {
        let Some(attribute_end) = code[attribute_start..]
            .find(']')
            .map(|relative| attribute_start + relative)
        else {
            continue;
        };
        let attribute = &code[attribute_start..=attribute_end];
        if !cfg_item_is_disabled_in_production(attribute) {
            continue;
        }
        let mut item_start = attribute_end + 1;
        loop {
            let trimmed = code[item_start..].trim_start();
            item_start = code.len() - trimmed.len();
            if !trimmed.starts_with("#[") {
                break;
            }
            let Some(end) = trimmed.find(']') else { break };
            item_start += end + 1;
        }
        let mut item = code[item_start..].trim_start();
        for prefix in [
            "pub(crate) ",
            "pub(super) ",
            "pub ",
            "async ",
            "unsafe ",
            "const ",
        ] {
            if let Some(rest) = item.strip_prefix(prefix) {
                item = rest.trim_start();
            }
        }
        if ![
            "fn ", "impl ", "struct ", "enum ", "mod ", "trait ", "union ",
        ]
        .iter()
        .any(|prefix| item.starts_with(prefix))
        {
            continue;
        }
        let Some(open) = code[item_start..]
            .find('{')
            .map(|relative| item_start + relative)
        else {
            continue;
        };
        let mut depth = 0usize;
        for (relative, byte) in code.as_bytes()[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        ranges.push(attribute_start..open + relative + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    ranges
}

fn unreviewed_mutation_calls(
    source_root: &Path,
    inventory: &[ProducerSite],
    sinks: &[&str],
    source_filter: Option<&BTreeSet<String>>,
) -> Vec<String> {
    let mut unreviewed = Vec::new();
    for path in rust_source_files(source_root) {
        let source = fs::read_to_string(&path).unwrap();
        let code = rust_code_only(&source);
        let nonproduction_ranges = nonproduction_cfg_ranges(&code);
        let relative = path
            .strip_prefix(source_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if source_filter.is_some_and(|filter| !filter.contains(&relative)) {
            continue;
        }
        for sink in sinks {
            let mut reviewed_ranges = inventory
                .iter()
                .filter(|site| {
                    site.source == relative && site.sinks.iter().any(|item| item == sink)
                })
                .map(|site| rust_function_range(&source, &site.function))
                .collect::<Vec<_>>();
            reviewed_ranges.extend(
                REVIEWED_INTERNAL_MUTATION_CALLERS
                    .iter()
                    .filter(|(reviewed_source, _, reviewed_sink)| {
                        *reviewed_source == relative && *reviewed_sink == *sink
                    })
                    .map(|(_, function, _)| rust_function_range(&source, function)),
            );
            for offset in rust_call_offsets(&code, sink) {
                if nonproduction_ranges
                    .iter()
                    .any(|range| range.contains(&offset))
                {
                    continue;
                }
                if reviewed_ranges.iter().any(|range| range.contains(&offset)) {
                    continue;
                }
                let line = source[..offset]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                let function = enclosing_rust_function(&code, offset).unwrap_or("<module>");
                unreviewed.push(format!("{relative}:{line} in `{function}` calls `{sink}`"));
            }
        }
    }
    unreviewed.sort();
    unreviewed
}

fn raw_mutation_call_counts(source_root: &Path) -> BTreeMap<(String, String, String), usize> {
    let mut calls = BTreeMap::new();
    for path in rust_source_files(&source_root.join("db")) {
        let source = fs::read_to_string(&path).unwrap();
        let code = rust_code_only(&source);
        let nonproduction_ranges = nonproduction_cfg_ranges(&code);
        let relative = path
            .strip_prefix(source_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for sink in RAW_MUTATION_SINKS {
            for offset in rust_call_offsets(&code, sink) {
                if nonproduction_ranges
                    .iter()
                    .any(|range| range.contains(&offset))
                {
                    continue;
                }
                let function = enclosing_rust_function(&code, offset).unwrap_or("<module>");
                *calls
                    .entry((relative.clone(), function.to_string(), (*sink).to_string()))
                    .or_insert(0) += 1;
            }
        }
        for (offset, sink) in mutating_open_options_call_offsets(&code) {
            if nonproduction_ranges
                .iter()
                .any(|range| range.contains(&offset))
            {
                continue;
            }
            let function = enclosing_rust_function(&code, offset).unwrap_or("<module>");
            *calls
                .entry((relative.clone(), function.to_string(), sink.to_string()))
                .or_insert(0) += 1;
        }
    }
    calls
}

fn unreviewed_raw_mutation_calls(
    source_root: &Path,
    reviewed: &[ReviewedRawMutation],
) -> Vec<String> {
    let actual = raw_mutation_call_counts(source_root);
    let mut expected = BTreeMap::new();
    for row in reviewed {
        assert!(
            row.count > 0,
            "raw mutation count must be positive: {row:?}"
        );
        assert!(
            RAW_MUTATION_SINKS.contains(&row.sink.as_str())
                || OPEN_OPTIONS_MUTATION_SINKS
                    .iter()
                    .any(|(_, sink)| *sink == row.sink.as_str()),
            "raw mutation inventory uses unknown sink: {row:?}"
        );
        assert!(
            expected
                .insert(
                    (row.source.clone(), row.function.clone(), row.sink.clone()),
                    row.count,
                )
                .is_none(),
            "duplicate raw mutation inventory row: {row:?}"
        );
    }
    let keys = actual
        .keys()
        .chain(expected.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    keys.into_iter()
        .filter_map(|(source, function, sink)| {
            let actual_count = actual
                .get(&(source.clone(), function.clone(), sink.clone()))
                .copied()
                .unwrap_or(0);
            let expected_count = expected
                .get(&(source.clone(), function.clone(), sink.clone()))
                .copied()
                .unwrap_or(0);
            (actual_count != expected_count).then(|| {
                format!(
                    "{source}|{function}|{sink}: expected {expected_count}, found {actual_count}"
                )
            })
        })
        .collect()
}

fn annotated_sites(source_root: &Path) -> BTreeMap<String, (String, String, String)> {
    let mut discovered = BTreeMap::new();
    let mut stack = vec![source_root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap();
            for line in source.lines().filter(|line| line.contains(PRODUCER_TAG)) {
                let annotation = line.split_once(PRODUCER_TAG).unwrap().1.trim();
                let fields = annotation.split_whitespace().collect::<Vec<_>>();
                assert_eq!(
                    fields.len(),
                    3,
                    "producer annotation must be `<site> <producer> <controlled|exempt>`: {}:{}",
                    path.display(),
                    line
                );
                let relative = path
                    .strip_prefix(source_root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                assert!(
                    discovered
                        .insert(
                            fields[0].to_string(),
                            (relative, fields[1].to_string(), fields[2].to_string())
                        )
                        .is_none(),
                    "duplicate producer site id `{}`",
                    fields[0]
                );
            }
        }
    }
    discovered
}

#[test]
fn every_filesystem_producer_entry_calls_its_reviewed_protocol() {
    let source_root = manifest_dir().join("src");
    let inventory = producer_inventory();
    let mut ids = BTreeSet::new();
    let mut uncovered = Vec::new();
    for site in &inventory {
        assert!(
            ids.insert(site.site.clone()),
            "duplicate inventory site `{}`",
            site.site
        );
        let source = fs::read_to_string(source_root.join(&site.source)).unwrap();
        let body = rust_function_body(&source, &site.function);
        let code = rust_code_only(body);
        for sink in &site.sinks {
            assert!(
                contains_rust_call(&code, sink),
                "inventoried mutation sink `{sink}` disappeared from {}::{}; review and update the concrete producer inventory",
                site.source,
                site.function
            );
        }
        match site.status.as_str() {
            "controlled" => {
                if !contains_rust_call(&code, &site.protocol) {
                    uncovered.push(format!(
                        "controlled producer `{}` reaches {:?} but does not call `{}` inside {}::{}",
                        site.site, site.sinks, site.protocol, site.source, site.function
                    ));
                }
                let tag = format!("{PRODUCER_TAG} {} {} controlled", site.site, site.producer);
                assert!(
                    body.contains(&tag),
                    "missing exact producer annotation `{tag}`"
                );
            }
            "exempt" => {
                assert!(
                    matches!(
                        site.producer.as_str(),
                        "exempt_rescue_output" | "exempt_alternate_output"
                    ),
                    "unreviewed exemption class `{}`",
                    site.producer
                );
                let tag = format!("{PRODUCER_TAG} {} {} exempt", site.site, site.producer);
                assert!(
                    body.contains(&tag),
                    "missing exact exemption annotation `{tag}`"
                );
            }
            "pending_task12" | "pending_task12_boundary" => {
                assert_eq!(site.producer, "CowPublication");
                assert_eq!(site.protocol, "task12_qualified_view_journal");
                assert!(
                    !code.contains("run_projection_alignment")
                        && !code.contains("run_ref_advancing_projection"),
                    "Task 12 mounted callback `{}` must not be accidentally treated as Task 11 clean authority",
                    site.site
                );
                if site.status == "pending_task12_boundary" {
                    let tag = format!(
                        "{PRODUCER_TAG} {} {} task12_callback_boundary",
                        site.site, site.producer
                    );
                    assert!(
                        body.contains(&tag),
                        "missing exact Task 12 boundary `{tag}`"
                    );
                }
            }
            other => panic!("unknown producer inventory status `{other}`"),
        }
    }
    let annotations = annotated_sites(&source_root.join("db"));
    let inventoried_annotations = inventory
        .iter()
        .filter(|site| site.status != "pending_task12")
        .map(|site| site.site.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        annotations.keys().cloned().collect::<BTreeSet<_>>(),
        inventoried_annotations,
        "producer annotations and the concrete reviewed inventory diverged"
    );
    assert!(
        uncovered.is_empty(),
        "uncovered changed-path producers:\n{}",
        uncovered.join("\n")
    );
    let unreviewed_calls =
        unreviewed_mutation_calls(&source_root, &inventory, DISCOVERED_MUTATION_SINKS, None);
    assert!(
        unreviewed_calls.is_empty(),
        "unreviewed direct filesystem mutation producers:\n{}",
        unreviewed_calls.join("\n")
    );
    let raw_inventory = raw_mutation_inventory();
    assert!(
        raw_inventory
            .iter()
            .filter(|row| row.class == "pending_task12")
            .all(|row| {
                row.source.starts_with("db/lane/workdir/view_")
                    || matches!(
                        row.source.as_str(),
                        "db/lane/workdir/dokan.rs"
                            | "db/lane/workdir/fuse.rs"
                            | "db/lane/workdir/nfs_overlay.rs"
                    )
            }),
        "Task 12 raw mutation boundaries must remain isolated to mounted-view helpers"
    );
    let unreviewed_raw_calls = unreviewed_raw_mutation_calls(&source_root, &raw_inventory);
    assert!(
        unreviewed_raw_calls.is_empty(),
        "unreviewed raw filesystem mutation producers:\n{}",
        unreviewed_raw_calls.join("\n")
    );
}

#[test]
fn producer_inventory_ignores_protocol_names_in_comments_and_literals() {
    let fake = r##"
        fn fake_producer() {
            // run_projection_alignment(fake_sink());
            let _comment = "run_ref_advancing_projection(other_sink())";
            let _raw = r#"materialize_lane_root_staged()"#;
            reviewed_sink();
        }
    "##;
    let body = rust_function_body(fake, "fake_producer");
    let code = rust_code_only(body);
    assert!(!code.contains("run_projection_alignment"));
    assert!(!code.contains("run_ref_advancing_projection"));
    assert!(!code.contains("materialize_lane_root_staged"));
    assert!(contains_rust_call(&code, "reviewed_sink"));
}

#[test]
fn producer_discovery_rejects_an_untagged_direct_mutation_sink() {
    let source_root = tempfile::tempdir().unwrap();
    fs::create_dir(source_root.path().join("db")).unwrap();
    fs::write(
        source_root.path().join("db/fake.rs"),
        r#"
            fn reviewed() {
                run_projection_alignment(|| materialize_files());
            }

            fn newly_added_untagged_producer() {
                materialize_files();
            }
        "#,
    )
    .unwrap();
    let inventory = vec![ProducerSite {
        status: "controlled".into(),
        source: "db/fake.rs".into(),
        function: "reviewed".into(),
        site: "reviewed".into(),
        producer: "Checkout".into(),
        protocol: "run_projection_alignment".into(),
        sinks: vec!["materialize_files".into()],
    }];

    let unreviewed =
        unreviewed_mutation_calls(source_root.path(), &inventory, &["materialize_files"], None);
    assert_eq!(unreviewed.len(), 1, "unexpected discovery: {unreviewed:?}");
    assert!(unreviewed[0].contains("db/fake.rs"));
    assert!(unreviewed[0].contains("calls `materialize_files`"));
}

#[test]
fn producer_discovery_rejects_new_rewind_materialization_and_raw_sinks() {
    let source_root = tempfile::tempdir().unwrap();
    fs::create_dir(source_root.path().join("db")).unwrap();
    fs::write(
        source_root.path().join("db/fake.rs"),
        r#"
            fn reviewed() {
                run_ref_advancing_projection(|| apply_rewind_workdir_projection());
            }

            fn unreviewed_projection() {
                apply_rewind_workdir_projection();
                materialize_files_at();
                fs::rename("before", "after");
            }
        "#,
    )
    .unwrap();
    let inventory = vec![ProducerSite {
        status: "controlled".into(),
        source: "db/fake.rs".into(),
        function: "reviewed".into(),
        site: "reviewed-rewind".into(),
        producer: "RestoreProjection".into(),
        protocol: "run_ref_advancing_projection".into(),
        sinks: vec!["apply_rewind_workdir_projection".into()],
    }];
    let unreviewed = unreviewed_mutation_calls(
        source_root.path(),
        &inventory,
        &[
            "apply_rewind_workdir_projection",
            "materialize_files_at",
            "fs::rename",
        ],
        None,
    );
    assert_eq!(unreviewed.len(), 3, "unexpected discovery: {unreviewed:?}");
    assert!(unreviewed
        .iter()
        .any(|call| call.contains("apply_rewind_workdir_projection")));
    assert!(unreviewed
        .iter()
        .any(|call| call.contains("materialize_files_at")));
    assert!(unreviewed.iter().any(|call| call.contains("fs::rename")));
}

#[test]
fn raw_producer_discovery_rejects_a_new_uninventoried_db_source_file() {
    let source_root = tempfile::tempdir().unwrap();
    fs::create_dir(source_root.path().join("db")).unwrap();
    fs::write(
        source_root.path().join("db/reviewed.rs"),
        r#"
            fn reviewed() {
                fs::write("reviewed", b"reviewed").unwrap();
            }
        "#,
    )
    .unwrap();
    fs::write(
        source_root.path().join("db/new_producer.rs"),
        r#"
            fn newly_added_uninventoried_producer() {
                fs::write("unreviewed", b"unreviewed").unwrap();
            }
        "#,
    )
    .unwrap();
    let inventory = vec![ReviewedRawMutation {
        class: "reviewed".into(),
        source: "db/reviewed.rs".into(),
        function: "reviewed".into(),
        sink: "fs::write".into(),
        count: 1,
    }];

    let unreviewed = unreviewed_raw_mutation_calls(source_root.path(), &inventory);
    assert_eq!(unreviewed.len(), 1, "unexpected discovery: {unreviewed:?}");
    assert!(unreviewed[0].contains("db/new_producer.rs"));
    assert!(unreviewed[0].contains("fs::write"));
    assert!(unreviewed[0].contains("expected 0, found 1"));
}

#[test]
fn raw_producer_discovery_rejects_an_extra_call_inside_a_reviewed_function() {
    let source_root = tempfile::tempdir().unwrap();
    fs::create_dir(source_root.path().join("db")).unwrap();
    fs::write(
        source_root.path().join("db/reviewed.rs"),
        r#"
            fn reviewed() {
                fs::write("first", b"first").unwrap();
                fs::write("second", b"second").unwrap();
            }
        "#,
    )
    .unwrap();
    let inventory = vec![ReviewedRawMutation {
        class: "reviewed".into(),
        source: "db/reviewed.rs".into(),
        function: "reviewed".into(),
        sink: "fs::write".into(),
        count: 1,
    }];

    let unreviewed = unreviewed_raw_mutation_calls(source_root.path(), &inventory);
    assert_eq!(unreviewed.len(), 1, "unexpected discovery: {unreviewed:?}");
    assert!(unreviewed[0].contains("db/reviewed.rs|reviewed|fs::write"));
    assert!(unreviewed[0].contains("expected 1, found 2"));
}

#[test]
fn raw_producer_discovery_covers_remove_dir_file_create_and_open_options_chains() {
    let source_root = tempfile::tempdir().unwrap();
    fs::create_dir(source_root.path().join("db")).unwrap();
    fs::write(
        source_root.path().join("db/raw_apis.rs"),
        r#"
            fn remove_one_directory() { fs::remove_dir("dir").unwrap(); }
            fn create_one_file() { File::create("file").unwrap(); }
            fn open_with_create() { OpenOptions::new().create(true).open("file").unwrap(); }
            fn open_with_write() { OpenOptions::new().write(true).open("file").unwrap(); }
            fn open_with_truncate() { OpenOptions::new().truncate(true).open("file").unwrap(); }
            fn open_with_create_new() { OpenOptions::new().create_new(true).open("file").unwrap(); }
            fn open_with_append() { OpenOptions::new().append(true).open("file").unwrap(); }
            fn open_with_conditional_write(enabled: bool) {
                OpenOptions::new().write(false || enabled).open("file").unwrap();
            }
            fn open_with_named_builder() {
                let mut options = OpenOptions::new();
                options.write(true).truncate(true);
                options.open("file").unwrap();
            }
            fn read_only() { OpenOptions::new().read(true).write(false).open("file").unwrap(); }
        "#,
    )
    .unwrap();

    let unreviewed = unreviewed_raw_mutation_calls(source_root.path(), &[]);
    for expected in [
        "remove_one_directory|fs::remove_dir",
        "create_one_file|File::create",
        "open_with_create|OpenOptions::create",
        "open_with_write|OpenOptions::write",
        "open_with_truncate|OpenOptions::truncate",
        "open_with_create_new|OpenOptions::create_new",
        "open_with_append|OpenOptions::append",
        "open_with_conditional_write|OpenOptions::write",
        "open_with_named_builder|OpenOptions::write",
        "open_with_named_builder|OpenOptions::truncate",
    ] {
        assert!(
            unreviewed.iter().any(|call| call.contains(expected)),
            "missing `{expected}` from {unreviewed:?}"
        );
    }
    assert_eq!(unreviewed.len(), 10, "unexpected discovery: {unreviewed:?}");
}

#[test]
fn raw_producer_discovery_scans_production_capable_cfg_items() {
    let source_root = tempfile::tempdir().unwrap();
    fs::create_dir(source_root.path().join("db")).unwrap();
    fs::write(
        source_root.path().join("db/cfg.rs"),
        r#"
            #[cfg(not(test))]
            fn enabled_outside_tests() { fs::write("production", b"production").unwrap(); }

            #[cfg(any(test, unix))]
            fn enabled_in_some_production_builds() { fs::write("unix", b"unix").unwrap(); }

            #[cfg(debug_assertions)]
            fn enabled_in_debug_production_builds() { fs::write("debug", b"debug").unwrap(); }

            #[cfg(test = "custom")]
            fn custom_test_key_value_is_not_the_test_harness() {
                fs::write("custom-test", b"custom-test").unwrap();
            }

            #[cfg(test)]
            fn test_only() { fs::write("test", b"test").unwrap(); }

            #[cfg(all(test, unix))]
            fn still_test_only() { fs::write("test-unix", b"test-unix").unwrap(); }
        "#,
    )
    .unwrap();

    let unreviewed = unreviewed_raw_mutation_calls(source_root.path(), &[]);
    assert_eq!(unreviewed.len(), 4, "unexpected discovery: {unreviewed:?}");
    assert!(unreviewed
        .iter()
        .any(|call| call.contains("enabled_outside_tests|fs::write")));
    assert!(unreviewed
        .iter()
        .any(|call| call.contains("enabled_in_some_production_builds|fs::write")));
    assert!(unreviewed
        .iter()
        .any(|call| call.contains("enabled_in_debug_production_builds|fs::write")));
    assert!(unreviewed
        .iter()
        .any(|call| call.contains("custom_test_key_value_is_not_the_test_harness|fs::write")));
    assert!(!unreviewed.iter().any(|call| call.contains("test_only")));
    assert!(!unreviewed
        .iter()
        .any(|call| call.contains("still_test_only")));
}

#[test]
fn raw_producer_discovery_does_not_skip_a_tests_rs_source_file() {
    let source_root = tempfile::tempdir().unwrap();
    fs::create_dir(source_root.path().join("db")).unwrap();
    fs::write(
        source_root.path().join("db/tests.rs"),
        r#"
            fn uninventoried_tests_rs_producer() {
                fs::write("unreviewed", b"unreviewed").unwrap();
            }
        "#,
    )
    .unwrap();

    let unreviewed = unreviewed_raw_mutation_calls(source_root.path(), &[]);
    assert_eq!(unreviewed.len(), 1, "unexpected discovery: {unreviewed:?}");
    assert!(unreviewed[0].contains("db/tests.rs|uninventoried_tests_rs_producer|fs::write"));
}

#[test]
fn controlled_rewind_sink_is_inside_the_ref_advancing_projection() {
    let rewind = fs::read_to_string(manifest_dir().join("src/db/lane/rewind.rs")).unwrap();
    let entry = rust_code_only(rust_function_body(&rewind, "rewind_lane"));
    let protocol = entry.find("run_ref_advancing_projection").unwrap();
    let controlled_interval = entry
        .find("with_materialized_lane_controlled_interval")
        .unwrap();
    let controlled_sink = entry.find("apply_rewind_workdir_projection").unwrap();
    let fallback_commit = entry.find("commit_lane_operation_atomic").unwrap();
    assert_eq!(
        rust_call_offsets(&entry, "apply_rewind_workdir_projection").len(),
        1
    );
    assert!(
        protocol < controlled_interval
            && controlled_interval < controlled_sink
            && controlled_sink < fallback_commit
    );

    let helper = rust_code_only(rust_function_body(
        &rewind,
        "apply_rewind_workdir_projection",
    ));
    assert!(contains_rust_call(&helper, "materialize_files_at"));
}

#[test]
fn every_materialized_lane_clean_consumer_uses_the_exact_snapshot_gateway() {
    let source_root = manifest_dir().join("src/db");
    let merge = fs::read_to_string(source_root.join("merge/lane.rs")).unwrap();
    let record = fs::read_to_string(source_root.join("lane/workdir/record.rs")).unwrap();
    let patch = fs::read_to_string(source_root.join("lane/patching.rs")).unwrap();
    let identity = fs::read_to_string(source_root.join("lane/identity.rs")).unwrap();
    let readiness = fs::read_to_string(source_root.join("lane/readiness.rs")).unwrap();

    let changed_paths = rust_code_only(rust_function_body(&merge, "lane_workdir_changed_paths"));
    assert!(
        contains_rust_call(&changed_paths, "compare_materialized_lane_candidates")
            || contains_rust_call(
                &changed_paths,
                "with_materialized_lane_authoritative_snapshot",
            ),
        "lane status/merge cleanliness still bypasses the exact materialized-lane snapshot"
    );
    let record_candidates = rust_code_only(rust_function_body(
        &record,
        "lane_workdir_record_changed_paths_with_case_fold",
    ));
    assert!(
        contains_rust_call(&record_candidates, "compare_materialized_lane_candidates")
            || contains_rust_call(
                &record_candidates,
                "with_materialized_lane_authoritative_snapshot",
            ),
        "lane record and normal preview still bypass the exact materialized-lane snapshot"
    );
    let patch_entry = rust_code_only(rust_function_body(&patch, "apply_lane_patch_locked"));
    assert!(
        contains_rust_call(&patch_entry, "with_materialized_lane_controlled_interval")
            && contains_rust_call(&patch_entry, "compare_controlled_projection_target"),
        "structured-patch authority no longer verifies its Prepared projection through the shared controlled interval"
    );
    let patch_apply = rust_code_only(rust_function_body(
        &patch,
        "apply_lane_patch_workdir_projection",
    ));
    assert!(
        patch_apply.contains("allow_legacy_manifest_shortcut")
            && contains_rust_call(
                &patch_apply,
                "refresh_clean_materialized_workdir_for_lane_patch",
            ),
        "legacy manifest refresh is no longer explicitly isolated behind the authority-off apply decision"
    );

    assert!(
        rust_function_body(&identity, "lane_status").contains("lane_workdir_changed_paths"),
        "lane status no longer uses the reviewed lane clean gateway"
    );
    assert!(
        rust_function_body(&record, "preview_lane_workdir_record")
            .contains("lane_workdir_record_changed_paths_with_case_fold"),
        "normal lane preview no longer uses the reviewed lane candidate gateway"
    );
    let record_entry = rust_code_only(rust_function_body(&record, "record_lane_workdir_locked"));
    assert!(
        contains_rust_call(
            &record_entry,
            "lane_workdir_record_changed_paths_with_case_fold",
        ) || contains_rust_call(&record_entry, "compare_materialized_lane_candidates")
            || contains_rust_call(
                &record_entry,
                "with_materialized_lane_authoritative_snapshot",
            ),
        "lane record bypasses the reviewed lane candidate gateway"
    );
    assert!(
        rust_function_body(&readiness, "lane_readiness").contains("lane_status"),
        "lane readiness no longer inherits the exact status snapshot"
    );
    assert!(
        rust_function_body(&merge, "ensure_lane_workdir_clean")
            .contains("lane_workdir_changed_paths"),
        "merge cleanliness no longer inherits the exact lane snapshot"
    );
}

fn init_lane_fixture(lane: &str, sparse: bool) -> (tempfile::TempDir, Trail, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn base() {}\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let spawn_result = if sparse {
        db.spawn_lane_with_workdir_paths(
            lane,
            Some("main"),
            true,
            None,
            None,
            None,
            &["README.md".to_string()],
        )
    } else {
        db.spawn_lane(lane, Some("main"), true, None, None)
    };
    let lane_root = match spawn_result {
        Ok(spawned) => PathBuf::from(spawned.workdir.unwrap()),
        Err(Error::OperationCommittedRepairRequired {
            operation,
            repair,
            reason,
        }) if matches!(
            repair.as_str(),
            "initial materialized lane ledger reconciliation"
                | "initial materialized lane ledger alignment"
        ) =>
        {
            // Association and materialization committed before this bounded
            // observer-boundary repair failure. Resolve that committed lane
            // through the public authoritative reconciliation path; never
            // retry creation or accept unrelated committed failures.
            let details = db.lane_details(lane).unwrap_or_else(|recovery| {
                panic!(
                    "lane spawn operation {operation} committed but `{repair}` could not resolve its association: {reason}; recovery: {recovery}"
                )
            });
            let workdir = PathBuf::from(details.branch.workdir.unwrap_or_else(|| {
                panic!(
                    "lane spawn operation {operation} committed but `{repair}` left no materialized workdir: {reason}"
                )
            }));
            let status = db.lane_status(lane).unwrap_or_else(|recovery| {
                panic!(
                    "lane spawn operation {operation} committed but `{repair}` reconciliation failed: {reason}; recovery: {recovery}"
                )
            });
            assert_eq!(
                status.workdir_state,
                Some(WorktreeState::Clean),
                "committed lane spawn reconciliation did not restore clean authority"
            );
            assert!(status.workdir_changed_paths.is_empty());
            let repaired = read_marker(&workdir);
            let resolved = db.lane_details(lane).unwrap().branch;
            assert_eq!(repaired["version"], 2);
            assert_eq!(repaired["root_id"], resolved.head_root.0);
            workdir
        }
        Err(error) => panic!("failed to initialize lane fixture `{lane}`: {error}"),
    };
    (temp, db, lane_root)
}

fn read_marker(workdir: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(workdir.join(".trail/workdir-manifest.json")).unwrap())
        .unwrap()
}

#[test]
fn materialized_lane_marker_is_compact_v2_and_sparse_selection_is_separate() {
    let _authority = AuthorityGuard::enabled();
    let (_temp, db, workdir) = init_lane_fixture("marker-sparse", true);
    let marker = read_marker(&workdir);
    assert_eq!(marker["version"], 2);
    for field in [
        "scope_id",
        "filesystem_identity",
        "ref_name",
        "ref_generation",
        "root_id",
        "policy_fingerprint",
        "epoch",
        "provider_cut",
        "provider_segment_id",
        "sparse_selection_fingerprint",
    ] {
        assert!(!marker[field].is_null(), "v2 marker is missing `{field}`");
    }
    assert!(
        marker.get("files").is_none(),
        "v2 marker regressed to an N-entry manifest"
    );

    let sparse: serde_json::Value =
        serde_json::from_slice(&fs::read(workdir.join(".trail/sparse-selection.json")).unwrap())
            .unwrap();
    assert_eq!(sparse["materialized_paths"][0], "README.md");
    let status = db.lane_status("marker-sparse").unwrap();
    assert_eq!(
        status.workdir_state,
        Some(WorktreeState::Clean),
        "fresh sparse authoritative status reported {:?}",
        status.workdir_changed_paths
    );

    fs::write(workdir.join("new.txt"), "visible sparse addition\n").unwrap();
    let dirty = db.lane_status("marker-sparse").unwrap();
    assert_eq!(dirty.workdir_state, Some(WorktreeState::DirtyUntracked));
    assert_eq!(dirty.workdir_changed_paths.len(), 1);
    assert_eq!(dirty.workdir_changed_paths[0].path, "new.txt");
    assert!(
        dirty
            .workdir_changed_paths
            .iter()
            .all(|change| change.path != "src/lib.rs"),
        "unmaterialized baseline path was reported as deleted"
    );
}

#[test]
fn malformed_sparse_selection_cannot_reconcile_or_publish_clean_authority() {
    let _authority = AuthorityGuard::enabled();
    for (suffix, malformed, expected_reason) in [
        (
            "missing-paths",
            serde_json::json!({}),
            "required `materialized_paths` field is missing",
        ),
        (
            "non-array",
            serde_json::json!({"version": 1, "materialized_paths": "README.md"}),
            "`materialized_paths` must be an array",
        ),
        (
            "mixed-types",
            serde_json::json!({"version": 1, "materialized_paths": ["README.md", 7]}),
            "`materialized_paths[1]` must be a string",
        ),
    ] {
        let lane = format!("sparse-malformed-{suffix}");
        let (_temp, db, workdir) = init_lane_fixture(&lane, true);
        let marker_path = workdir.join(".trail/workdir-manifest.json");
        let marker_before = fs::read(&marker_path).unwrap();
        fs::remove_file(workdir.join("README.md")).unwrap();
        fs::write(
            workdir.join(".trail/sparse-selection.json"),
            serde_json::to_vec(&malformed).unwrap(),
        )
        .unwrap();

        let error = db.lane_status(&lane).unwrap_err();
        assert_eq!(error.code(), "DATABASE_CORRUPT");
        assert!(error.to_string().contains(expected_reason));
        assert_eq!(
            fs::read(marker_path).unwrap(),
            marker_before,
            "malformed sparse selection published a replacement clean marker"
        );
    }
}

#[test]
fn explicit_empty_sparse_selection_excludes_baseline_absence_but_not_visible_additions() {
    let _authority = AuthorityGuard::enabled();
    let (_temp, db, workdir) = init_lane_fixture("sparse-explicit-empty", true);
    let marker_before = read_marker(&workdir);
    fs::remove_file(workdir.join("README.md")).unwrap();
    fs::write(
        workdir.join(".trail/sparse-selection.json"),
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "materialized_paths": []
        }))
        .unwrap(),
    )
    .unwrap();

    let clean = db.lane_status("sparse-explicit-empty").unwrap();
    assert_eq!(clean.workdir_state, Some(WorktreeState::Clean));
    assert!(clean.workdir_changed_paths.is_empty());
    let marker_after = read_marker(&workdir);
    assert_eq!(marker_after["version"], 2);
    assert_ne!(
        marker_after["sparse_selection_fingerprint"], marker_before["sparse_selection_fingerprint"],
        "explicit empty selection was confused with the prior selected path set"
    );

    fs::write(workdir.join("visible.txt"), "visible addition\n").unwrap();
    let dirty = db.lane_status("sparse-explicit-empty").unwrap();
    assert_eq!(dirty.workdir_state, Some(WorktreeState::DirtyUntracked));
    assert_eq!(dirty.workdir_changed_paths.len(), 1);
    assert_eq!(dirty.workdir_changed_paths[0].path, "visible.txt");
}

#[test]
fn authority_off_uses_direct_lane_observation_and_creates_no_trusted_scope() {
    let _authority = AuthorityGuard::disabled();
    let (temp, db, workdir) = init_lane_fixture("authority-off", false);
    let before = read_marker(&workdir);
    assert_ne!(
        before["version"], 2,
        "authority-off spawn wrote a V2 marker"
    );
    let status = db.lane_status("authority-off").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
    let after = read_marker(&workdir);
    assert_ne!(
        after["version"], 2,
        "authority-off status wrote a V2 marker"
    );
    assert!(
        after.get("files").is_some(),
        "authority-off status did not preserve the usable V1 manifest cache"
    );

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let trusted: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM changed_path_scopes
             WHERE scope_kind='materialized_lane' AND trust_state='trusted' AND retired_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        trusted, 0,
        "authority-off lane status persisted trusted scope state"
    );
}

#[test]
fn sparse_hydration_failure_restores_bytes_selection_and_v2_marker() {
    struct SparseSelectionFailure;
    impl SparseSelectionFailure {
        fn enable() -> Self {
            trail::test_support::set_sparse_selection_write_failure_for_current_thread(true);
            Self
        }
    }
    impl Drop for SparseSelectionFailure {
        fn drop(&mut self) {
            trail::test_support::set_sparse_selection_write_failure_for_current_thread(false);
        }
    }

    let _authority = AuthorityGuard::enabled();
    let (_temp, mut db, workdir) = init_lane_fixture("sparse-rollback-ledger", true);
    let initial_status = db.lane_status("sparse-rollback-ledger").unwrap();
    assert_eq!(
        initial_status.workdir_state,
        Some(WorktreeState::Clean),
        "fresh sparse authoritative status reported {:?}",
        initial_status.workdir_changed_paths
    );
    let metadata_dir = workdir.join(".trail");
    let selection = metadata_dir.join("sparse-selection.json");
    let marker = metadata_dir.join("workdir-manifest.json");
    let selection_before = fs::read(&selection).unwrap();
    let marker_before = fs::read(&marker).unwrap();
    assert!(!workdir.join("src/lib.rs").exists());

    let result = {
        let _failure = SparseSelectionFailure::enable();
        db.sync_lane_workdir_with_paths(
            "sparse-rollback-ledger",
            false,
            &["src/lib.rs".to_string()],
        )
    };

    assert!(
        result.is_err(),
        "injected sparse-selection failure unexpectedly committed"
    );
    assert!(!workdir.join("src/lib.rs").exists());
    assert_eq!(fs::read(&selection).unwrap(), selection_before);
    assert_eq!(fs::read(&marker).unwrap(), marker_before);
    assert_eq!(
        db.lane_status("sparse-rollback-ledger")
            .unwrap()
            .workdir_state,
        Some(WorktreeState::Clean)
    );
}

#[test]
fn legacy_future_and_mismatched_lane_markers_reconcile_before_clean() {
    let _authority = AuthorityGuard::enabled();
    let (_temp, db, workdir) = init_lane_fixture("marker-reconcile", false);
    let marker_path = workdir.join(".trail/workdir-manifest.json");
    let clean_head = db
        .lane_details("marker-reconcile")
        .unwrap()
        .branch
        .head_root;

    let invalid_markers = [
        serde_json::json!({"version": 1, "root_id": clean_head.0, "files": {}}),
        serde_json::json!({"version": 99}),
        {
            let mut marker = read_marker(&workdir);
            marker["root_id"] = serde_json::Value::String("sha256:wrong-root".into());
            marker
        },
        {
            let mut marker = read_marker(&workdir);
            marker["filesystem_identity"] = serde_json::json!([0, 1, 2]);
            marker
        },
    ];

    for invalid in invalid_markers {
        fs::write(&marker_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let status = db.lane_status("marker-reconcile").unwrap();
        assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
        let repaired = read_marker(&workdir);
        assert_eq!(repaired["version"], 2);
        assert_eq!(repaired["root_id"], clean_head.0);
        assert!(repaired.get("files").is_none());
    }
}

#[test]
fn observer_owner_loss_reconciles_lane_and_preserves_dirty_evidence() {
    let _authority = AuthorityGuard::enabled();
    let (temp, db, workdir) = init_lane_fixture("owner-loss", false);
    assert_eq!(
        db.lane_status("owner-loss").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let lane_id = db.lane_details("owner-loss").unwrap().branch.lane_id;
    let scope_id: String = conn
        .query_row(
            "SELECT scope_id FROM changed_path_scopes
             WHERE scope_kind='materialized_lane' AND owner_id=?1 AND retired_at IS NULL",
            params![lane_id],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "DELETE FROM changed_path_observer_owners WHERE scope_id=?1",
        params![scope_id],
    )
    .unwrap();
    fs::write(workdir.join("README.md"), "external after owner loss\n").unwrap();

    let status = db.lane_status("owner-loss").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::DirtyTracked));
    assert_eq!(status.workdir_changed_paths.len(), 1);
    assert_eq!(status.workdir_changed_paths[0].path, "README.md");
}

#[test]
fn public_lane_preview_record_and_status_share_one_authoritative_lane_scope() {
    let _authority = AuthorityGuard::enabled();
    let (_temp, mut db, workdir) = init_lane_fixture("public-record", false);
    fs::write(workdir.join("README.md"), "recorded through lane ledger\n").unwrap();

    let preview = db.preview_lane_workdir_record("public-record").unwrap();
    assert!(!preview.clean);
    assert_eq!(preview.changed_paths.len(), 1);
    assert_eq!(preview.changed_paths[0].path, "README.md");

    let recorded = db
        .record_lane_workdir("public-record", Some("authoritative lane record".into()))
        .unwrap();
    assert!(recorded.operation.is_some());
    assert_eq!(recorded.changed_paths.len(), 1);
    assert_eq!(recorded.changed_paths[0].path, "README.md");
    let status = db.lane_status("public-record").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
    assert!(status.workdir_changed_paths.is_empty());
    assert_eq!(read_marker(&workdir)["root_id"], recorded.root_id.0);
}

#[test]
fn observed_lane_record_builds_from_fenced_bytes_and_retains_same_path_after_c2() {
    let _authority = AuthorityGuard::enabled();
    let (_temp, mut db, workdir) = init_lane_fixture("record-fenced-bytes", false);
    let captured = b"captured before c2\n";
    let after_c2 = b"written after c2\n";
    fs::write(workdir.join("README.md"), captured).unwrap();
    trail::test_support::install_lane_record_after_c2_write_for_current_thread(
        workdir.join("README.md"),
        after_c2.to_vec(),
    );

    let recorded = db
        .record_lane_workdir("record-fenced-bytes", Some("fenced bytes".into()))
        .unwrap();
    let recorded_file = db
        .inspect_root(&recorded.root_id.0)
        .unwrap()
        .files
        .into_iter()
        .find(|file| file.path == "README.md")
        .unwrap();
    assert_eq!(
        recorded_file.content_hash,
        hex::encode(Sha256::digest(captured)),
        "lane record reread the same path after c2 instead of using fenced bytes"
    );
    assert_eq!(fs::read(workdir.join("README.md")).unwrap(), after_c2);

    let status = db.lane_status("record-fenced-bytes").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::DirtyTracked));
    assert_eq!(status.workdir_changed_paths.len(), 1);
    assert_eq!(status.workdir_changed_paths[0].path, "README.md");
}

#[test]
fn observed_lane_record_postcommit_failures_require_committed_repair() {
    let _authority = AuthorityGuard::enabled();
    for boundary in ["manifest", "event", "turn", "marker"] {
        let (temp, mut db, workdir) =
            init_lane_fixture(&format!("record-postcommit-{boundary}"), false);
        let before = db
            .lane_details(&format!("record-postcommit-{boundary}"))
            .unwrap()
            .branch;
        fs::write(
            workdir.join("README.md"),
            format!("changed before {boundary}\n"),
        )
        .unwrap();
        trail::test_support::set_lane_record_postcommit_failure_for_current_thread(Some(boundary));
        let error = db
            .record_lane_workdir(
                &format!("record-postcommit-{boundary}"),
                Some(format!("postcommit {boundary}")),
            )
            .unwrap_err();
        trail::test_support::set_lane_record_postcommit_failure_for_current_thread(None);

        assert_eq!(error.code(), "COMMITTED_REPAIR_REQUIRED");
        let Error::OperationCommittedRepairRequired {
            operation,
            repair,
            reason,
        } = error
        else {
            unreachable!()
        };
        assert!(!operation.is_empty());
        assert!(repair.contains(boundary));
        assert!(reason.contains(boundary));

        let after = db
            .lane_details(&format!("record-postcommit-{boundary}"))
            .unwrap()
            .branch;
        assert_ne!(after.head_change, before.head_change);
        assert_ne!(after.head_root, before.head_root);
        let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
        let ledger_root: String = conn
            .query_row(
                "SELECT baseline_root_id FROM changed_path_scopes
                 WHERE scope_kind='materialized_lane' AND retired_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ledger_root, after.head_root.0);
    }
}

#[test]
fn patch_and_rewind_publish_ref_lane_and_compact_marker_at_one_visible_boundary() {
    let _authority = AuthorityGuard::enabled();
    let (_temp, mut db, workdir) = init_lane_fixture("projection-boundary", false);
    let base = db
        .lane_details("projection-boundary")
        .unwrap()
        .branch
        .head_change;
    let mut patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "projection boundary",
        "edits": [{"op": "write", "path": "README.md", "content": "patched\n"}]
    }))
    .unwrap();
    patch.base_change = Some(base.0.clone());
    let applied = db.apply_lane_patch("projection-boundary", patch).unwrap();
    let after_patch = db.lane_details("projection-boundary").unwrap();
    assert_eq!(after_patch.branch.head_change, applied.operation);
    assert_eq!(after_patch.branch.head_root, applied.root_id);
    assert_eq!(read_marker(&workdir)["root_id"], applied.root_id.0);
    assert_eq!(
        fs::read_to_string(workdir.join("README.md")).unwrap(),
        "patched\n"
    );

    let rewound = db
        .rewind_lane("projection-boundary", &base.0, false, true)
        .unwrap();
    let after_rewind = db.lane_details("projection-boundary").unwrap();
    assert_eq!(after_rewind.branch.head_change, rewound.operation);
    assert_eq!(after_rewind.branch.head_root, rewound.root_id);
    assert_eq!(read_marker(&workdir)["root_id"], rewound.root_id.0);
    assert_eq!(
        fs::read_to_string(workdir.join("README.md")).unwrap(),
        "base\n"
    );
}

#[test]
fn mutable_materialized_lane_authority_pins_the_lane_root_and_reconciles_markers() {
    let _authority = AuthorityGuard::enabled();
    trail::test_support::changed_path_materialized_lane_snapshot_flow().unwrap();
}

#[test]
fn producer_publication_retains_later_same_path_events_and_matches_full_scan_oracle() {
    let _authority = AuthorityGuard::enabled();
    trail::test_support::changed_path_intent_acknowledgement_race().unwrap();
    trail::test_support::changed_path_reconciliation_races().unwrap();
    trail::test_support::changed_path_reconciliation_oracle().unwrap();
}
