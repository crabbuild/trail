mod common;

use common::configured_prolly;

#[test]
fn put_get_delete_and_range_are_ordered() {
    let prolly = configured_prolly();
    let mut tree = prolly.create();

    tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
    tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
    tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();

    assert_eq!(prolly.get(&tree, b"a").unwrap(), Some(b"1".to_vec()));
    assert_eq!(prolly.get(&tree, b"missing").unwrap(), None);

    let entries = prolly
        .range(&tree, b"a", Some(b"c"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        entries,
        vec![
            (b"a".to_vec(), b"1".to_vec()),
            (b"b".to_vec(), b"2".to_vec())
        ]
    );

    let tree = prolly.delete(&tree, b"b").unwrap();
    assert_eq!(prolly.get(&tree, b"b").unwrap(), None);
}

#[test]
fn updates_are_immutable_and_content_addressed() {
    let prolly = configured_prolly();
    let base = prolly.create();
    let first = prolly.put(&base, b"key".to_vec(), b"old".to_vec()).unwrap();
    let second = prolly
        .put(&first, b"key".to_vec(), b"new".to_vec())
        .unwrap();

    assert_eq!(prolly.get(&first, b"key").unwrap(), Some(b"old".to_vec()));
    assert_eq!(prolly.get(&second, b"key").unwrap(), Some(b"new".to_vec()));
    assert_ne!(first.root, second.root);
}

#[test]
fn same_content_builds_same_root() {
    let left = configured_prolly();
    let right = configured_prolly();

    let mut left_tree = left.create();
    let mut right_tree = right.create();
    for (key, value) in [(b"a", b"1"), (b"b", b"2"), (b"c", b"3"), (b"d", b"4")] {
        left_tree = left.put(&left_tree, key.to_vec(), value.to_vec()).unwrap();
        right_tree = right
            .put(&right_tree, key.to_vec(), value.to_vec())
            .unwrap();
    }

    assert_eq!(left_tree.root, right_tree.root);
}

#[test]
fn node_cache_can_be_observed_and_cleared() {
    let prolly = configured_prolly();
    let tree = prolly.create();
    let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();

    assert!(prolly.cache_len() > 0);
    assert_eq!(prolly.get(&tree, b"a").unwrap(), Some(b"1".to_vec()));

    prolly.clear_cache();
    assert_eq!(prolly.cache_len(), 0);
    assert_eq!(prolly.get(&tree, b"a").unwrap(), Some(b"1".to_vec()));
    assert!(prolly.cache_len() > 0);
}
