mod common;

use common::{canonical_diffs, configured_prolly};
use prolly::{Diff, Error};

#[test]
fn diff_and_three_way_merge_work_for_disjoint_changes() {
    let prolly = configured_prolly();

    let base = prolly.create();
    let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
    let left = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
    let right = prolly.put(&base, b"c".to_vec(), b"3".to_vec()).unwrap();

    assert_eq!(
        prolly.diff(&base, &left).unwrap(),
        vec![Diff::Added {
            key: b"b".to_vec(),
            val: b"2".to_vec()
        }]
    );

    let merged = prolly.merge(&base, &left, &right, None).unwrap();
    assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
    assert_eq!(prolly.get(&merged, b"b").unwrap(), Some(b"2".to_vec()));
    assert_eq!(prolly.get(&merged, b"c").unwrap(), Some(b"3".to_vec()));
}

#[test]
fn conflicting_three_way_merge_requires_resolver() {
    let prolly = configured_prolly();

    let base = prolly.create();
    let base = prolly.put(&base, b"k".to_vec(), b"base".to_vec()).unwrap();
    let left = prolly.put(&base, b"k".to_vec(), b"left".to_vec()).unwrap();
    let right = prolly.put(&base, b"k".to_vec(), b"right".to_vec()).unwrap();

    assert!(matches!(
        prolly.merge(&base, &left, &right, None),
        Err(Error::Conflict(_))
    ));

    let merged = prolly
        .merge(
            &base,
            &left,
            &right,
            Some(Box::new(|conflict| {
                let mut value = conflict.left.clone();
                value.extend_from_slice(b"+");
                value.extend_from_slice(&conflict.right);
                Some(value)
            })),
        )
        .unwrap();
    assert_eq!(
        prolly.get(&merged, b"k").unwrap(),
        Some(b"left+right".to_vec())
    );
}

#[test]
fn streaming_diff_matches_eager_diff_content() {
    let prolly = configured_prolly();
    let mut base = prolly.create();
    for i in 0..24 {
        base = prolly
            .put(
                &base,
                format!("k{i:02}").into_bytes(),
                format!("base-{i}").into_bytes(),
            )
            .unwrap();
    }

    let mut other = base.clone();
    other = prolly
        .put(&other, b"k03".to_vec(), b"changed".to_vec())
        .unwrap();
    other = prolly.delete(&other, b"k05").unwrap();
    other = prolly
        .put(&other, b"k99".to_vec(), b"added".to_vec())
        .unwrap();

    let eager = prolly.diff(&base, &other).unwrap();
    let streaming = prolly
        .stream_diff(&base, &other)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(canonical_diffs(streaming), canonical_diffs(eager));
}
