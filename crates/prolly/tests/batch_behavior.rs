mod common;

use std::sync::Arc;

use common::{configured_prolly, entries};
use prolly::{BatchBuilder, Config, MemStore, Mutation, Prolly};

#[test]
fn batch_mutations_apply_in_one_importable_api() {
    let prolly = configured_prolly();
    let tree = prolly.create();

    let tree = prolly
        .batch(
            &tree,
            vec![
                Mutation::Upsert {
                    key: b"a".to_vec(),
                    val: b"1".to_vec(),
                },
                Mutation::Upsert {
                    key: b"b".to_vec(),
                    val: b"2".to_vec(),
                },
                Mutation::Delete {
                    key: b"missing".to_vec(),
                },
            ],
        )
        .unwrap();

    assert_eq!(prolly.get(&tree, b"a").unwrap(), Some(b"1".to_vec()));
    assert_eq!(prolly.get(&tree, b"b").unwrap(), Some(b"2".to_vec()));
}

#[test]
fn batch_builder_matches_incremental_tree_for_unsorted_input() {
    let store = Arc::new(MemStore::new());
    let config = Config::builder()
        .min_chunk_size(2)
        .max_chunk_size(5)
        .chunking_factor(2)
        .hash_seed(7)
        .build();

    let mut builder = BatchBuilder::new(store.clone(), config.clone());
    for i in (0..64).rev() {
        builder.add(
            format!("key-{i:03}").into_bytes(),
            format!("value-{i}").into_bytes(),
        );
    }
    let built = builder.build().unwrap();

    let prolly = Prolly::new(store, config);
    let mut incremental = prolly.create();
    for i in 0..64 {
        incremental = prolly
            .put(
                &incremental,
                format!("key-{i:03}").into_bytes(),
                format!("value-{i}").into_bytes(),
            )
            .unwrap();
    }

    assert_eq!(entries(&prolly, &built), entries(&prolly, &incremental));
    assert!(built.root.is_some());
    assert!(incremental.root.is_some());
}

#[test]
fn batch_mutations_are_last_write_wins_and_match_repeated_ops() {
    let batched = configured_prolly();
    let mut base = batched.create();
    base = batched.put(&base, b"a".to_vec(), b"old".to_vec()).unwrap();
    base = batched.put(&base, b"b".to_vec(), b"old".to_vec()).unwrap();

    let mutations = vec![
        Mutation::Upsert {
            key: b"b".to_vec(),
            val: b"first".to_vec(),
        },
        Mutation::Delete { key: b"a".to_vec() },
        Mutation::Upsert {
            key: b"c".to_vec(),
            val: b"3".to_vec(),
        },
        Mutation::Upsert {
            key: b"b".to_vec(),
            val: b"second".to_vec(),
        },
    ];

    let batched_tree = batched.batch(&base, mutations).unwrap();

    let repeated = configured_prolly();
    let mut expected = repeated.create();
    expected = repeated
        .put(&expected, b"a".to_vec(), b"old".to_vec())
        .unwrap();
    expected = repeated
        .put(&expected, b"b".to_vec(), b"old".to_vec())
        .unwrap();
    expected = repeated.delete(&expected, b"a").unwrap();
    expected = repeated
        .put(&expected, b"c".to_vec(), b"3".to_vec())
        .unwrap();
    expected = repeated
        .put(&expected, b"b".to_vec(), b"second".to_vec())
        .unwrap();

    assert_eq!(
        entries(&batched, &batched_tree),
        entries(&repeated, &expected)
    );
}
