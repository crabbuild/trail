use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::ffi::{sqlite3_auto_extension, SQLITE_OK};
use rusqlite::{params, Connection};
use sqlite_vec::sqlite3_vec_init;

const DEFAULT_ROWS: usize = 10_000;
const DEFAULT_DIMS: usize = 128;
const DEFAULT_QUERIES: usize = 25;
const DEFAULT_TOP_K: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VectorBackend {
    SqliteVec0,
    ExactBlobScan,
}

impl VectorBackend {
    fn from_env() -> Self {
        match std::env::var("TRAIL_SQLITE_VECTOR_BACKEND")
            .unwrap_or_else(|_| "sqlite_vec0".to_string())
            .as_str()
        {
            "exact" | "sqlite_exact_blob_scan" => Self::ExactBlobScan,
            "vec0" | "sqlite_vec" | "sqlite_vec0" | "sqlite_vector_extension" => Self::SqliteVec0,
            other => {
                panic!("unsupported TRAIL_SQLITE_VECTOR_BACKEND={other}; use sqlite_vec0 or exact")
            }
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::SqliteVec0 => "sqlite_vec0",
            Self::ExactBlobScan => "sqlite_exact_blob_scan",
        }
    }

    fn uses_sqlite_vec(self) -> bool {
        matches!(self, Self::SqliteVec0)
    }
}

fn main() {
    let backend = VectorBackend::from_env();
    let rows = env_usize("TRAIL_SQLITE_VECTOR_ROWS").unwrap_or(DEFAULT_ROWS);
    let dims = env_usize("TRAIL_SQLITE_VECTOR_DIMS").unwrap_or(DEFAULT_DIMS);
    let queries = env_usize("TRAIL_SQLITE_VECTOR_QUERIES").unwrap_or(DEFAULT_QUERIES);
    let top_k = env_usize("TRAIL_SQLITE_VECTOR_TOP_K").unwrap_or(DEFAULT_TOP_K);
    let keep_db = std::env::var("TRAIL_SQLITE_VECTOR_KEEP_DB").ok().as_deref() == Some("1");
    let path = db_path();

    remove_sqlite_files(&path);
    if backend.uses_sqlite_vec() {
        register_sqlite_vec_extension();
    }

    println!("sqlite vector memory bench");
    println!("db_path={}", path.display());
    println!("rows={rows}");
    println!("dims={dims}");
    println!("queries={queries}");
    println!("top_k={top_k}");
    println!("backend={}", backend.name());

    let conn = Connection::open(&path).unwrap();
    apply_pragmas(&conn);
    if backend.uses_sqlite_vec() {
        println!("sqlite_vec_version={}", sqlite_vec_version(&conn));
    }
    println!(
        "operation,rows,dims,queries,top_k,total_ms,avg_ms,rows_per_sec,db_bytes,verified,status"
    );
    create_schema(&conn, dims, backend);

    let insert_start = Instant::now();
    insert_memory_rows(&conn, rows, dims, backend);
    let insert_elapsed = insert_start.elapsed();
    let db_bytes = sqlite_db_bytes(&path);
    print_row(
        "insert_embeddings",
        rows,
        dims,
        1,
        top_k,
        insert_elapsed,
        rows,
        db_bytes,
        verify_row_count(&conn, rows),
        "ok",
    );

    let query = embedding_for_index(rows / 3, dims);
    let scope_start = Instant::now();
    let mut last_scope_report = SearchReport::default();
    for _ in 0..queries {
        last_scope_report = search(
            &conn,
            &query,
            top_k,
            Some(("lane", "lane-002")),
            None,
            backend,
        );
    }
    let scope_elapsed = scope_start.elapsed();
    let db_bytes = sqlite_db_bytes(&path);
    print_row(
        "search_scope",
        rows,
        dims,
        queries,
        top_k,
        scope_elapsed,
        last_scope_report.scanned_rows * queries,
        db_bytes,
        verify_ranked_results(&last_scope_report.results, top_k),
        "ok",
    );

    let path_start = Instant::now();
    let mut last_path_report = SearchReport::default();
    for _ in 0..queries {
        last_path_report = search(&conn, &query, top_k, None, Some("src/auth"), backend);
    }
    let path_elapsed = path_start.elapsed();
    let db_bytes = sqlite_db_bytes(&path);
    print_row(
        "search_path_prefix",
        rows,
        dims,
        queries,
        top_k,
        path_elapsed,
        last_path_report.scanned_rows * queries,
        db_bytes,
        verify_ranked_results(&last_path_report.results, top_k),
        "ok",
    );

    let context_start = Instant::now();
    let mut context_rows = 0usize;
    let mut context_scanned_rows = 0usize;
    for _ in 0..queries {
        let report = search(
            &conn,
            &query,
            top_k,
            Some(("workspace", "default")),
            None,
            backend,
        );
        context_scanned_rows += report.scanned_rows;
        context_rows += context_packet(&conn, &report.results).len();
    }
    let context_elapsed = context_start.elapsed();
    let db_bytes = sqlite_db_bytes(&path);
    print_row(
        "context_packet",
        rows,
        dims,
        queries,
        top_k,
        context_elapsed,
        context_scanned_rows,
        db_bytes,
        context_rows > 0,
        "ok",
    );

    drop(conn);
    if !keep_db {
        remove_sqlite_files(&path);
    }
}

#[derive(Debug, Clone)]
struct SearchResult {
    memory_id: String,
    distance: f32,
}

#[derive(Default)]
struct SearchReport {
    results: Vec<SearchResult>,
    scanned_rows: usize,
}

fn register_sqlite_vec_extension() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // sqlite-vec exposes a C extension entrypoint; register it before opening benchmark connections.
        let result = unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *const i8,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> i32,
            >(sqlite3_vec_init as *const ())))
        };
        assert_eq!(result, SQLITE_OK, "failed to register sqlite-vec extension");
    });
}

fn sqlite_vec_version(conn: &Connection) -> String {
    conn.query_row("SELECT vec_version()", [], |row| row.get(0))
        .unwrap()
}

fn create_schema(conn: &Connection, dims: usize, backend: VectorBackend) {
    conn.execute_batch(
        "\
        CREATE TABLE memory_items (
            memory_ord INTEGER UNIQUE NOT NULL,
            memory_id TEXT PRIMARY KEY,
            scope_type TEXT NOT NULL,
            scope_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            path TEXT,
            body TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE memory_embeddings (
            memory_id TEXT PRIMARY KEY REFERENCES memory_items(memory_id),
            memory_ord INTEGER UNIQUE NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            dims INTEGER NOT NULL,
            embedding BLOB NOT NULL
        );
        CREATE INDEX memory_items_scope_idx ON memory_items(scope_type, scope_id, status);
        CREATE INDEX memory_items_path_idx ON memory_items(path, status);
        ",
    )
    .unwrap();

    if backend.uses_sqlite_vec() {
        conn.execute_batch(&format!(
            "\
            CREATE VIRTUAL TABLE memory_embeddings_vec USING vec0(
                memory_ord INTEGER PRIMARY KEY,
                embedding float[{dims}] distance_metric=cosine,
                scope_type TEXT,
                scope_id TEXT,
                path TEXT,
                status TEXT
            );
            "
        ))
        .unwrap();
    }
}

fn insert_memory_rows(conn: &Connection, rows: usize, dims: usize, backend: VectorBackend) {
    let tx = conn.unchecked_transaction().unwrap();
    {
        let mut item_stmt = tx
            .prepare_cached(
                "\
                INSERT INTO memory_items
                    (memory_ord, memory_id, scope_type, scope_id, kind, path, body, status, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', ?8)
                ",
            )
            .unwrap();
        let mut embedding_stmt = tx
            .prepare_cached(
                "\
                INSERT INTO memory_embeddings
                    (memory_id, memory_ord, provider, model, dims, embedding)
                VALUES (?1, ?2, 'synthetic', 'deterministic', ?3, ?4)
                ",
            )
            .unwrap();
        let mut vec_stmt = if backend.uses_sqlite_vec() {
            Some(
                tx.prepare_cached(
                    "\
                    INSERT INTO memory_embeddings_vec
                        (memory_ord, embedding, scope_type, scope_id, path, status)
                    VALUES (?1, ?2, ?3, ?4, ?5, 'active')
                    ",
                )
                .unwrap(),
            )
        } else {
            None
        };

        for i in 0..rows {
            let memory_ord = (i + 1) as i64;
            let memory_id = format!("mem_{i:012}");
            let (scope_type, scope_id) = scope_for_index(i);
            let path = path_for_index(i);
            let body = format!("Synthetic memory item {i} for {scope_type}/{scope_id} at {path}");
            let embedding = encode_f32_vec(&embedding_for_index(i, dims));

            item_stmt
                .execute(params![
                    memory_ord,
                    &memory_id,
                    scope_type,
                    &scope_id,
                    kind_for_index(i),
                    &path,
                    &body,
                    i as i64
                ])
                .unwrap();
            embedding_stmt
                .execute(params![&memory_id, memory_ord, dims as i64, &embedding])
                .unwrap();
            if let Some(stmt) = vec_stmt.as_mut() {
                stmt.execute(params![
                    memory_ord, &embedding, scope_type, &scope_id, &path
                ])
                .unwrap();
            }
        }
    }
    tx.commit().unwrap();
}

fn search(
    conn: &Connection,
    query: &[f32],
    top_k: usize,
    scope: Option<(&str, &str)>,
    path_prefix: Option<&str>,
    backend: VectorBackend,
) -> SearchReport {
    match backend {
        VectorBackend::SqliteVec0 => sqlite_vec_search(conn, query, top_k, scope, path_prefix),
        VectorBackend::ExactBlobScan => exact_search(conn, query, top_k, scope, path_prefix),
    }
}

fn sqlite_vec_search(
    conn: &Connection,
    query: &[f32],
    top_k: usize,
    scope: Option<(&str, &str)>,
    path_prefix: Option<&str>,
) -> SearchReport {
    let query = encode_f32_vec(query);
    let mut results = Vec::new();

    if let Some((scope_type, scope_id)) = scope {
        let mut stmt = conn
            .prepare_cached(
                "\
                WITH matches AS (
                    SELECT memory_ord, distance
                    FROM memory_embeddings_vec
                    WHERE embedding MATCH ?1
                      AND k = ?2
                      AND status = 'active'
                      AND scope_type = ?3
                      AND scope_id = ?4
                )
                SELECT i.memory_id, matches.distance
                FROM matches
                JOIN memory_items i ON i.memory_ord = matches.memory_ord
                ORDER BY matches.distance
                ",
            )
            .unwrap();
        let mut rows = stmt
            .query(params![query, top_k as i64, scope_type, scope_id])
            .unwrap();
        while let Some(row) = rows.next().unwrap() {
            results.push(SearchResult {
                memory_id: row.get(0).unwrap(),
                distance: row.get(1).unwrap(),
            });
        }
    } else if let Some(prefix) = path_prefix {
        let end = prefix_upper_bound(prefix);
        let mut stmt = conn
            .prepare_cached(
                "\
                WITH matches AS (
                    SELECT memory_ord, distance
                    FROM memory_embeddings_vec
                    WHERE embedding MATCH ?1
                      AND k = ?2
                      AND status = 'active'
                      AND path >= ?3
                      AND path < ?4
                )
                SELECT i.memory_id, matches.distance
                FROM matches
                JOIN memory_items i ON i.memory_ord = matches.memory_ord
                ORDER BY matches.distance
                ",
            )
            .unwrap();
        let mut rows = stmt
            .query(params![query, top_k as i64, prefix, end])
            .unwrap();
        while let Some(row) = rows.next().unwrap() {
            results.push(SearchResult {
                memory_id: row.get(0).unwrap(),
                distance: row.get(1).unwrap(),
            });
        }
    } else {
        let mut stmt = conn
            .prepare_cached(
                "\
                WITH matches AS (
                    SELECT memory_ord, distance
                    FROM memory_embeddings_vec
                    WHERE embedding MATCH ?1
                      AND k = ?2
                      AND status = 'active'
                )
                SELECT i.memory_id, matches.distance
                FROM matches
                JOIN memory_items i ON i.memory_ord = matches.memory_ord
                ORDER BY matches.distance
                ",
            )
            .unwrap();
        let mut rows = stmt.query(params![query, top_k as i64]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            results.push(SearchResult {
                memory_id: row.get(0).unwrap(),
                distance: row.get(1).unwrap(),
            });
        }
    }

    let scanned_rows = results.len();
    SearchReport {
        results,
        scanned_rows,
    }
}

fn exact_search(
    conn: &Connection,
    query: &[f32],
    top_k: usize,
    scope: Option<(&str, &str)>,
    path_prefix: Option<&str>,
) -> SearchReport {
    let mut sql = "\
        SELECT e.memory_id, e.embedding
        FROM memory_embeddings e
        JOIN memory_items i ON i.memory_id = e.memory_id
        WHERE i.status = 'active'
    "
    .to_string();

    if scope.is_some() {
        sql.push_str(" AND i.scope_type = ?1 AND i.scope_id = ?2");
    } else if path_prefix.is_some() {
        sql.push_str(" AND i.path >= ?1 AND i.path < ?2");
    }

    let mut stmt = conn.prepare_cached(&sql).unwrap();
    let mut rows = if let Some((scope_type, scope_id)) = scope {
        stmt.query(params![scope_type, scope_id]).unwrap()
    } else if let Some(prefix) = path_prefix {
        let end = prefix_upper_bound(prefix);
        stmt.query(params![prefix, end]).unwrap()
    } else {
        stmt.query([]).unwrap()
    };

    let mut results = Vec::new();
    let mut scanned_rows = 0usize;
    while let Some(row) = rows.next().unwrap() {
        scanned_rows += 1;
        let memory_id: String = row.get(0).unwrap();
        let bytes: Vec<u8> = row.get(1).unwrap();
        let embedding = decode_f32_vec(&bytes);
        if embedding.len() == query.len() {
            results.push(SearchResult {
                memory_id,
                distance: cosine_distance(query, &embedding),
            });
        }
    }

    results.sort_by(|left, right| {
        left.distance
            .partial_cmp(&right.distance)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.memory_id.cmp(&right.memory_id))
    });
    results.truncate(top_k);
    SearchReport {
        results,
        scanned_rows,
    }
}

fn context_packet(conn: &Connection, results: &[SearchResult]) -> Vec<String> {
    let mut stmt = conn
        .prepare_cached(
            "\
            SELECT body
            FROM memory_items
            WHERE memory_id = ?1 AND status = 'active'
            ",
        )
        .unwrap();
    results
        .iter()
        .filter_map(|result| {
            stmt.query_row(params![result.memory_id], |row| row.get::<_, String>(0))
                .ok()
        })
        .collect()
}

fn verify_row_count(conn: &Connection, rows: usize) -> bool {
    let item_count: usize = conn
        .query_row("SELECT COUNT(*) FROM memory_items", [], |row| row.get(0))
        .unwrap();
    let embedding_count: usize = conn
        .query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })
        .unwrap();
    item_count == rows && embedding_count == rows
}

fn verify_ranked_results(results: &[SearchResult], top_k: usize) -> bool {
    !results.is_empty()
        && results.len() <= top_k
        && results
            .windows(2)
            .all(|pair| pair[0].distance <= pair[1].distance)
}

fn embedding_for_index(index: usize, dims: usize) -> Vec<f32> {
    let mut vector = Vec::with_capacity(dims);
    for dim in 0..dims {
        let raw = ((index.wrapping_mul(31) + dim.wrapping_mul(17)) % 997) as f32;
        vector.push((raw / 997.0) - 0.5);
    }
    normalize(vector)
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    1.0 - left
        .iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f32>()
}

fn encode_f32_vec(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(vector));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_f32_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn scope_for_index(index: usize) -> (&'static str, String) {
    match index % 5 {
        0 => ("workspace", "default".to_string()),
        1 => ("branch", "main".to_string()),
        2 => ("lane", format!("lane-{:03}", index % 10)),
        3 => ("session", format!("session-{:03}", index % 25)),
        _ => ("turn", format!("turn-{:03}", index % 50)),
    }
}

fn kind_for_index(index: usize) -> &'static str {
    match index % 6 {
        0 => "decision",
        1 => "observation",
        2 => "failed_attempt",
        3 => "constraint",
        4 => "test_result",
        _ => "reference",
    }
}

fn path_for_index(index: usize) -> String {
    match index % 4 {
        0 => format!("src/auth/file_{index:06}.rs"),
        1 => format!("src/storage/file_{index:06}.rs"),
        2 => format!("docs/agent/file_{index:06}.md"),
        _ => format!("tests/e2e/file_{index:06}.rs"),
    }
}

fn prefix_upper_bound(prefix: &str) -> String {
    let mut bytes = prefix.as_bytes().to_vec();
    for index in (0..bytes.len()).rev() {
        if bytes[index] != u8::MAX {
            bytes[index] += 1;
            bytes.truncate(index + 1);
            return String::from_utf8(bytes).unwrap();
        }
    }
    format!("{prefix}\u{10ffff}")
}

fn apply_pragmas(conn: &Connection) {
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "synchronous", "NORMAL").unwrap();
    conn.pragma_update(None, "temp_store", "MEMORY").unwrap();
}

#[allow(
    clippy::too_many_arguments,
    reason = "mirrors the fixed benchmark result columns"
)]
fn print_row(
    operation: &str,
    rows: usize,
    dims: usize,
    queries: usize,
    top_k: usize,
    elapsed: Duration,
    scanned_rows: usize,
    db_bytes: u64,
    verified: bool,
    status: &str,
) {
    let total_ms = elapsed.as_secs_f64() * 1_000.0;
    let avg_ms = total_ms / queries.max(1) as f64;
    let rows_per_sec = if total_ms > 0.0 {
        scanned_rows as f64 / (total_ms / 1_000.0)
    } else {
        0.0
    };
    println!(
        "{operation},{rows},{dims},{queries},{top_k},{total_ms:.3},{avg_ms:.3},{rows_per_sec:.0},{db_bytes},{verified},{status}"
    );
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok()
}

fn db_path() -> PathBuf {
    if let Ok(path) = std::env::var("TRAIL_SQLITE_VECTOR_DB") {
        return PathBuf::from(path);
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "trail-sqlite-vector-memory-{}-{nanos}.db",
        std::process::id()
    ))
}

fn sqlite_db_bytes(path: &Path) -> u64 {
    sqlite_paths(path)
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok().map(|metadata| metadata.len()))
        .sum()
}

fn sqlite_paths(path: &Path) -> [PathBuf; 3] {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ]
}

fn remove_sqlite_files(path: &Path) {
    for path in sqlite_paths(path) {
        let _ = std::fs::remove_file(path);
    }
}
