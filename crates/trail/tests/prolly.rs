use trail::prolly::{Config, MemStore, Prolly};
use trail::prolly_tree;

#[test]
fn prolly_is_importable_through_trail_namespaces() {
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
