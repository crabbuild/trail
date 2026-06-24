use crabdb::prolly::{Config, MemStore, Prolly};
use crabdb::prolly_tree;

#[test]
fn prolly_is_importable_through_crabdb_namespaces() {
    let store = MemStore::new();
    let prolly = Prolly::new(store, Config::default());
    let tree = prolly.create();
    let tree = prolly
        .put(&tree, b"module".to_vec(), b"works".to_vec())
        .unwrap();

    assert_eq!(
        prolly.get(&tree, b"module").unwrap(),
        Some(b"works".to_vec())
    );

    let _also_available: prolly_tree::Tree = tree;
}
