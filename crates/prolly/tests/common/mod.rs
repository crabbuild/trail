#![allow(dead_code)]

use std::sync::Arc;

use prolly::{Cid, Config, Diff, MemStore, Node, Prolly, Store, Tree};

pub fn configured_prolly() -> Prolly<MemStore> {
    let config = Config::builder()
        .min_chunk_size(2)
        .max_chunk_size(4)
        .chunking_factor(2)
        .build();
    Prolly::new(MemStore::new(), config)
}

pub fn entries<S: Store>(prolly: &Prolly<S>, tree: &Tree) -> Vec<(Vec<u8>, Vec<u8>)> {
    prolly
        .range(tree, &[], None)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

pub fn canonical_diffs(mut diffs: Vec<Diff>) -> Vec<Diff> {
    diffs.sort_by(|left, right| {
        let (left_key, left_kind) = diff_sort_parts(left);
        let (right_key, right_kind) = diff_sort_parts(right);
        left_key
            .cmp(right_key)
            .then_with(|| left_kind.cmp(&right_kind))
    });
    diffs
}

pub fn load_node(store: &Arc<MemStore>, cid: &Cid) -> Node {
    let bytes = store.get(cid.as_bytes()).unwrap().unwrap();
    Node::from_bytes(&bytes).unwrap()
}

pub fn assert_tree_invariants(store: &Arc<MemStore>, tree: &Tree, config: &Config) {
    if let Some(root) = &tree.root {
        let (_, first_key) = assert_node_invariants(store, root, None, config);
        assert!(first_key.is_some());
    }
}

fn diff_sort_parts(diff: &Diff) -> (&[u8], u8) {
    match diff {
        Diff::Added { key, .. } => (key, 0),
        Diff::Changed { key, .. } => (key, 1),
        Diff::Removed { key, .. } => (key, 2),
    }
}

fn assert_node_invariants(
    store: &Arc<MemStore>,
    cid: &Cid,
    expected_level: Option<u8>,
    config: &Config,
) -> (usize, Option<Vec<u8>>) {
    let node = load_node(store, cid);

    assert_eq!(node.keys.len(), node.vals.len());
    assert!(node.keys.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(node.len() <= config.max_chunk_size);
    if let Some(level) = expected_level {
        assert_eq!(node.level, level);
    }

    if node.leaf {
        return (1, node.keys.first().cloned());
    }

    let mut total = 1;
    for (key, child) in node.keys.iter().zip(&node.vals) {
        let child_cid = Cid(child.as_slice().try_into().unwrap());
        let (child_count, first_key) =
            assert_node_invariants(store, &child_cid, Some(node.level - 1), config);
        assert_eq!(Some(key), first_key.as_ref());
        total += child_count;
    }

    (total, node.keys.first().cloned())
}
