#![cfg(feature = "sqlite")]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use prolly::{BatchOp, Config, Prolly, SqliteStore, Store};

#[test]
fn sqlite_store_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<SqliteStore>();
}

#[test]
fn sqlite_store_supports_store_contract_operations() {
    let store = SqliteStore::open_in_memory().unwrap();

    store.put(b"a", b"1").unwrap();
    store.put(b"b", b"2").unwrap();
    assert_eq!(store.get(b"a").unwrap(), Some(b"1".to_vec()));

    let keys: Vec<&[u8]> = vec![b"b", b"missing", b"a"];
    assert_eq!(
        store.batch_get_ordered(&keys).unwrap(),
        vec![Some(b"2".to_vec()), None, Some(b"1".to_vec())]
    );

    store
        .batch(&[
            BatchOp::Upsert {
                key: b"a",
                value: b"updated",
            },
            BatchOp::Delete { key: b"b" },
            BatchOp::Upsert {
                key: b"c",
                value: b"3",
            },
        ])
        .unwrap();

    let found = store.batch_get(&[b"a", b"b", b"c"]).unwrap();
    assert_eq!(found.get(b"a".as_slice()), Some(&b"updated".to_vec()));
    assert!(!found.contains_key(b"b".as_slice()));
    assert_eq!(found.get(b"c".as_slice()), Some(&b"3".to_vec()));
}

#[test]
fn sqlite_store_persists_prolly_tree_nodes_across_reopen() {
    let path = temp_db_path("persist");
    remove_sqlite_files(&path);

    let config = Config::builder()
        .min_chunk_size(2)
        .max_chunk_size(4)
        .chunking_factor(2)
        .build();

    let tree = {
        let store = SqliteStore::open(&path).unwrap();
        let prolly = Prolly::new(store, config.clone());
        let mut tree = prolly.create();

        for i in 0..40 {
            tree = prolly
                .put(
                    &tree,
                    format!("k{i:02}").into_bytes(),
                    format!("v{i:02}").into_bytes(),
                )
                .unwrap();
        }

        tree
    };

    {
        let store = SqliteStore::open(&path).unwrap();
        let prolly = Prolly::new(store, config);

        assert_eq!(prolly.get(&tree, b"k00").unwrap(), Some(b"v00".to_vec()));
        assert_eq!(prolly.get(&tree, b"k17").unwrap(), Some(b"v17".to_vec()));
        assert_eq!(prolly.get(&tree, b"k39").unwrap(), Some(b"v39".to_vec()));
        assert_eq!(prolly.get(&tree, b"missing").unwrap(), None);
    }

    remove_sqlite_files(&path);
}

fn temp_db_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "crabdb-prolly-{label}-{}-{nanos}.db",
        std::process::id()
    ))
}

fn remove_sqlite_files(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("db-wal"));
    let _ = fs::remove_file(path.with_extension("db-shm"));
}
