use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use prolly::{append_batch, Config, Error, Mutation, Prolly, Resolver, SlateDbStore, Tree};
use slatedb::object_store::aws::AmazonS3Builder;
use slatedb::object_store::path::Path as ObjectPath;
use slatedb::object_store::{ObjectStore, ObjectStoreExt};

const DEFAULT_RECORDS: usize = 1_000_000;
const DEFAULT_CHANGES: usize = 10_000;
const DEFAULT_BUILD_BATCH: usize = 50_000;

#[derive(Debug, Default)]
struct ObjectStats {
    count: usize,
    bytes: u64,
}

fn main() {
    let records = env_usize("PROLLY_SLATEDB_OPS_RECORDS").unwrap_or(DEFAULT_RECORDS);
    let changes = env_usize("PROLLY_SLATEDB_OPS_CHANGES")
        .unwrap_or(DEFAULT_CHANGES)
        .min((records / 4).max(1));
    let build_batch = env_usize("PROLLY_SLATEDB_OPS_BUILD_BATCH").unwrap_or(DEFAULT_BUILD_BATCH);
    let path = db_path();
    let object_store = build_slatedb_object_store();

    remove_slatedb_prefix(object_store.clone(), &path);

    println!("slatedb operation suite bench");
    println!("path={path}");
    println!("endpoint={}", slatedb_endpoint());
    println!("bucket={}", slatedb_bucket());
    println!("records={records}");
    println!("changes={changes}");
    println!("build_batch={build_batch}");
    println!("operation,base_records,items,total_ms,items_per_sec,result_count,verified,status");

    let config = bench_config();
    let store = Arc::new(open_store(&path, object_store.clone()));
    let prolly = Prolly::new(store.clone(), config.clone());
    let base = build_base(&prolly, records, build_batch);
    let stats = object_stats(object_store.clone(), &path);
    println!(
        "object_stats_after_build,count={},bytes={}",
        stats.count, stats.bytes
    );

    measure_row("sample_get", records, 3, || {
        let verified = verify_base_samples(&prolly, &base, records);
        (3, verified)
    });

    measure_row("point_get_hot", records, changes, || {
        let mut found = 0usize;
        for i in 0..changes {
            let idx = i * 97 % records;
            if prolly.get(&base, &key_for_index(idx)).unwrap().as_deref()
                == Some(value_for_index(idx).as_slice())
            {
                found += 1;
            }
        }
        (found, found == changes)
    });

    let random_read_indices = random_indices(records, changes, 0x7265_6164);
    measure_row("point_get_random", records, changes, || {
        let found = random_read_indices
            .iter()
            .filter(|&&idx| {
                prolly.get(&base, &key_for_index(idx)).unwrap().as_deref()
                    == Some(value_for_index(idx).as_slice())
            })
            .count();
        (found, found == changes)
    });

    let clustered_read_indices = clustered_indices(records, changes, 8, 0x636c_7573_7465_72);
    measure_row("point_get_clustered", records, changes, || {
        let found = clustered_read_indices
            .iter()
            .filter(|&&idx| {
                prolly.get(&base, &key_for_index(idx)).unwrap().as_deref()
                    == Some(value_for_index(idx).as_slice())
            })
            .count();
        (found, found == changes)
    });

    measure_row("range_scan_window", records, changes, || {
        let start_idx = records / 3;
        let end_idx = start_idx + changes;
        let count = prolly
            .range(
                &base,
                &key_for_index(start_idx),
                Some(&key_for_index(end_idx)),
            )
            .unwrap()
            .map(|entry| entry.unwrap())
            .count();
        (count, count == changes)
    });

    measure_row("range_scan_full", records, records, || {
        let count = prolly
            .range(&base, &[], None)
            .unwrap()
            .map(|entry| entry.unwrap())
            .count();
        (count, count == records)
    });

    let update_ops = update_mutations(records, changes, "spread-update");
    let batch_updated = measure_tree_row("batch_update_spread", records, changes, || {
        prolly.batch(&base, update_ops.clone()).unwrap()
    });
    let verified = verify_updates(&prolly, &batch_updated, changes, "spread-update");
    println!("verify_batch_update_spread,{records},{changes},0.000,0,0,{verified},ok");

    let random_update_indices = random_indices(records, changes, 0x7570_6461_7465);
    let random_update_ops = update_mutations_for_indices(&random_update_indices, "random-update");
    let random_updated = measure_tree_row("batch_update_random", records, changes, || {
        prolly.batch(&base, random_update_ops.clone()).unwrap()
    });
    let verified = verify_updates_for_indices(
        &prolly,
        &random_updated,
        &random_update_indices,
        "random-update",
    );
    println!("verify_batch_update_random,{records},{changes},0.000,0,0,{verified},ok");

    let clustered_update_indices = clustered_indices(records, changes, 8, 0x7570_6461_7465_c1u64);
    let clustered_update_ops =
        update_mutations_for_indices(&clustered_update_indices, "cluster-update");
    let clustered_updated = measure_tree_row("batch_update_clustered", records, changes, || {
        prolly.batch(&base, clustered_update_ops.clone()).unwrap()
    });
    let verified = verify_updates_for_indices(
        &prolly,
        &clustered_updated,
        &clustered_update_indices,
        "cluster-update",
    );
    println!("verify_batch_update_clustered,{records},{changes},0.000,0,0,{verified},ok");

    let delete_ops = delete_mutations(records, changes);
    let batch_deleted = measure_tree_row("batch_delete_spread", records, changes, || {
        prolly.batch(&base, delete_ops.clone()).unwrap()
    });
    let verified = verify_deletes(&prolly, &batch_deleted, changes);
    println!("verify_batch_delete_spread,{records},{changes},0.000,0,0,{verified},ok");

    let random_delete_indices = random_indices(records, changes, 0x6465_6c65_7465);
    let random_delete_ops = delete_mutations_for_indices(&random_delete_indices);
    let random_deleted = measure_tree_row("batch_delete_random", records, changes, || {
        prolly.batch(&base, random_delete_ops.clone()).unwrap()
    });
    let verified = verify_deletes_for_indices(&prolly, &random_deleted, &random_delete_indices);
    println!("verify_batch_delete_random,{records},{changes},0.000,0,0,{verified},ok");

    let clustered_delete_indices = clustered_indices(records, changes, 8, 0x6465_6c65_7465_c1u64);
    let clustered_delete_ops = delete_mutations_for_indices(&clustered_delete_indices);
    let clustered_deleted = measure_tree_row("batch_delete_clustered", records, changes, || {
        prolly.batch(&base, clustered_delete_ops.clone()).unwrap()
    });
    let verified =
        verify_deletes_for_indices(&prolly, &clustered_deleted, &clustered_delete_indices);
    println!("verify_batch_delete_clustered,{records},{changes},0.000,0,0,{verified},ok");

    let mixed_ops = mixed_mutations(records, changes, "mixed");
    let mixed = measure_tree_row("batch_mixed_spread", records, changes, || {
        prolly.batch(&base, mixed_ops.clone()).unwrap()
    });
    let verified = verify_mixed(&prolly, &mixed, records, changes, "mixed");
    println!("verify_batch_mixed_spread,{records},{changes},0.000,0,0,{verified},ok");

    let random_mixed_indices = random_indices(records, changes, 0x6d69_7865_64);
    let random_mixed_ops = mixed_mutations_for_indices(&random_mixed_indices, "random-mixed");
    let random_mixed = measure_tree_row("batch_mixed_random", records, changes, || {
        prolly.batch(&base, random_mixed_ops.clone()).unwrap()
    });
    let verified = verify_mixed_for_indices(
        &prolly,
        &random_mixed,
        &random_mixed_indices,
        "random-mixed",
    );
    println!("verify_batch_mixed_random,{records},{changes},0.000,0,0,{verified},ok");

    let clustered_mixed_indices = clustered_indices(records, changes, 8, 0x6d69_7865_64_c1u64);
    let clustered_mixed_ops =
        mixed_mutations_for_indices(&clustered_mixed_indices, "cluster-mixed");
    let clustered_mixed = measure_tree_row("batch_mixed_clustered", records, changes, || {
        prolly.batch(&base, clustered_mixed_ops.clone()).unwrap()
    });
    let verified = verify_mixed_for_indices(
        &prolly,
        &clustered_mixed,
        &clustered_mixed_indices,
        "cluster-mixed",
    );
    println!("verify_batch_mixed_clustered,{records},{changes},0.000,0,0,{verified},ok");

    let append_ops = append_mutations(records, changes, "append");
    let appended = measure_tree_row("append_suffix", records, changes, || {
        append_batch(&prolly, &base, append_ops.clone()).unwrap()
    });
    let verified = verify_appends(&prolly, &appended, records, changes, "append");
    println!("verify_append_suffix,{records},{changes},0.000,0,0,{verified},ok");

    measure_row("diff_identical", records, 1, || {
        let diffs = prolly.diff(&base, &base).unwrap();
        let count = diffs.len();
        (count, count == 0)
    });

    measure_row("diff_sparse_update", records, changes, || {
        let diffs = prolly.diff(&base, &batch_updated).unwrap();
        let count = diffs.len();
        (count, count == changes)
    });

    measure_row("diff_random_update", records, changes, || {
        let diffs = prolly.diff(&base, &random_updated).unwrap();
        let count = diffs.len();
        (count, count == changes)
    });

    measure_row("diff_clustered_update", records, changes, || {
        let diffs = prolly.diff(&base, &clustered_updated).unwrap();
        let count = diffs.len();
        (count, count == changes)
    });

    measure_row("stream_diff_sparse_update", records, changes, || {
        let diffs = prolly
            .stream_diff(&base, &batch_updated)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let count = diffs.len();
        (count, count == changes)
    });

    measure_row("stream_diff_random_update", records, changes, || {
        let diffs = prolly
            .stream_diff(&base, &random_updated)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let count = diffs.len();
        (count, count == changes)
    });

    measure_row("stream_diff_clustered_update", records, changes, || {
        let diffs = prolly
            .stream_diff(&base, &clustered_updated)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let count = diffs.len();
        (count, count == changes)
    });

    measure_row("diff_append_suffix", records, changes, || {
        let diffs = prolly.diff(&base, &appended).unwrap();
        let count = diffs.len();
        (count, count == changes)
    });

    let start_idx = records / 3;
    let end_idx = start_idx + changes;
    let range_changed = prolly
        .batch(
            &base,
            range_changed_mutations(start_idx, end_idx, "range-change"),
        )
        .unwrap();
    measure_row("range_diff_window", records, changes, || {
        let diffs = prolly
            .range_diff(
                &base,
                &range_changed,
                &key_for_index(start_idx),
                Some(&key_for_index(end_idx)),
            )
            .unwrap();
        let count = diffs.len();
        (count, count > 0)
    });

    let cluster_window_start = records / 2;
    let cluster_window_len = changes.min(records - cluster_window_start);
    let cluster_window_end = cluster_window_start + cluster_window_len;
    let cluster_window_indices: Vec<_> = (cluster_window_start..cluster_window_end).collect();
    let cluster_window_changed = prolly
        .batch(
            &base,
            update_mutations_for_indices(&cluster_window_indices, "cluster-window"),
        )
        .unwrap();
    measure_row(
        "range_diff_clustered_window",
        records,
        cluster_window_len,
        || {
            let diffs = prolly
                .range_diff(
                    &base,
                    &cluster_window_changed,
                    &key_for_index(cluster_window_start),
                    Some(&key_for_index(cluster_window_end)),
                )
                .unwrap();
            let count = diffs.len();
            (count, count == cluster_window_len)
        },
    );

    measure_row("diff_empty_to_full", records, records, || {
        let empty = prolly.create();
        let diffs = prolly.diff(&empty, &base).unwrap();
        let count = diffs.len();
        (count, count == records)
    });

    let left_indices = spread_indices_in_range(0, records / 2, changes, 7);
    let right_indices = spread_indices_in_range(records / 2, records, changes, 7);
    let left = prolly
        .batch(&base, update_mutations_for_indices(&left_indices, "left"))
        .unwrap();
    let right = prolly
        .batch(&base, update_mutations_for_indices(&right_indices, "right"))
        .unwrap();
    let merged = measure_tree_row("merge_spread_disjoint", records, changes * 2, || {
        prolly.merge(&base, &left, &right, None).unwrap()
    });
    let verified = verify_updates_for_indices(&prolly, &merged, &left_indices, "left")
        && verify_updates_for_indices(&prolly, &merged, &right_indices, "right");
    println!(
        "verify_merge_spread_disjoint,{records},{},0.000,0,0,{verified},ok",
        changes * 2
    );

    let random_left_indices = random_indices_in_range(0, records / 2, changes, 0x6c65_6674);
    let random_right_indices =
        random_indices_in_range(records / 2, records, changes, 0x7269_6768_74);
    let random_left = prolly
        .batch(
            &base,
            update_mutations_for_indices(&random_left_indices, "random-left"),
        )
        .unwrap();
    let random_right = prolly
        .batch(
            &base,
            update_mutations_for_indices(&random_right_indices, "random-right"),
        )
        .unwrap();
    let random_merged = measure_tree_row("merge_random_disjoint", records, changes * 2, || {
        prolly
            .merge(&base, &random_left, &random_right, None)
            .unwrap()
    });
    let verified =
        verify_updates_for_indices(&prolly, &random_merged, &random_left_indices, "random-left")
            && verify_updates_for_indices(
                &prolly,
                &random_merged,
                &random_right_indices,
                "random-right",
            );
    println!(
        "verify_merge_random_disjoint,{records},{},0.000,0,0,{verified},ok",
        changes * 2
    );

    let clustered_left_indices =
        clustered_indices_in_range(0, records / 3, changes, 4, 0x6c65_6674_c1u64);
    let clustered_right_indices =
        clustered_indices_in_range(records * 2 / 3, records, changes, 4, 0x7269_6768_74_c1u64);
    let clustered_left = prolly
        .batch(
            &base,
            update_mutations_for_indices(&clustered_left_indices, "cluster-left"),
        )
        .unwrap();
    let clustered_right = prolly
        .batch(
            &base,
            update_mutations_for_indices(&clustered_right_indices, "cluster-right"),
        )
        .unwrap();
    let clustered_merged =
        measure_tree_row("merge_clustered_disjoint", records, changes * 2, || {
            prolly
                .merge(&base, &clustered_left, &clustered_right, None)
                .unwrap()
        });
    let verified = verify_updates_for_indices(
        &prolly,
        &clustered_merged,
        &clustered_left_indices,
        "cluster-left",
    ) && verify_updates_for_indices(
        &prolly,
        &clustered_merged,
        &clustered_right_indices,
        "cluster-right",
    );
    println!(
        "verify_merge_clustered_disjoint,{records},{},0.000,0,0,{verified},ok",
        changes * 2
    );

    let conflict_count = changes.min(128);
    let conflict_left = prolly
        .batch(&base, conflict_mutations(conflict_count, "left-conflict"))
        .unwrap();
    let conflict_right = prolly
        .batch(&base, conflict_mutations(conflict_count, "right-conflict"))
        .unwrap();
    measure_row("merge_conflict_detect", records, conflict_count, || {
        let detected = matches!(
            prolly.merge(&base, &conflict_left, &conflict_right, None),
            Err(Error::Conflict(_))
        );
        (1, detected)
    });
    let resolver: Resolver = Box::new(|conflict| Some(conflict.right.clone()));
    let resolved = measure_tree_row("merge_conflict_resolved", records, conflict_count, || {
        prolly
            .merge(&base, &conflict_left, &conflict_right, Some(resolver))
            .unwrap()
    });
    let verified = verify_conflict_resolution(&prolly, &resolved, conflict_count);
    println!("verify_merge_conflict_resolved,{records},{conflict_count},0.000,0,0,{verified},ok");

    measure_row("stream_diff_sparse_update_cold", records, changes, || {
        prolly.clear_cache();
        let diffs = prolly
            .stream_diff(&base, &batch_updated)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let count = diffs.len();
        (count, count == changes)
    });

    measure_row("stream_diff_random_update_cold", records, changes, || {
        prolly.clear_cache();
        let diffs = prolly
            .stream_diff(&base, &random_updated)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let count = diffs.len();
        (count, count == changes)
    });

    measure_row(
        "stream_diff_clustered_update_cold",
        records,
        changes,
        || {
            prolly.clear_cache();
            let diffs = prolly
                .stream_diff(&base, &clustered_updated)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let count = diffs.len();
            (count, count == changes)
        },
    );

    let stats = object_stats(object_store.clone(), &path);
    println!(
        "object_stats_after_suite,count={},bytes={}",
        stats.count, stats.bytes
    );

    drop(prolly);
    drop(store);

    measure_row("reopen_final_sample_get", records, 3, || {
        let reopened = Prolly::new(
            Arc::new(open_store(&path, object_store.clone())),
            config.clone(),
        );
        let verified = verify_base_samples(&reopened, &base, records);
        (3, verified)
    });

    remove_slatedb_prefix(object_store, &path);
}

fn build_base<S: prolly::Store>(prolly: &Prolly<S>, records: usize, batch_size: usize) -> Tree {
    let mut tree = prolly.create();
    let start = Instant::now();
    let mut written = 0usize;
    while written < records {
        let count = (records - written).min(batch_size);
        tree = append_batch(prolly, &tree, base_mutations(written, count)).unwrap();
        written += count;
    }
    print_row(
        "build_base_append_batches",
        records,
        records,
        start.elapsed(),
        written,
        written == records,
        "ok",
    );
    tree
}

fn measure_tree_row<F>(operation: &str, records: usize, items: usize, f: F) -> Tree
where
    F: FnOnce() -> Tree,
{
    let start = Instant::now();
    let tree = f();
    print_row(
        operation,
        records,
        items,
        start.elapsed(),
        items,
        true,
        "ok",
    );
    tree
}

fn measure_row<F>(operation: &str, records: usize, items: usize, f: F)
where
    F: FnOnce() -> (usize, bool),
{
    let start = Instant::now();
    let (result_count, verified) = f();
    print_row(
        operation,
        records,
        items,
        start.elapsed(),
        result_count,
        verified,
        "ok",
    );
}

fn print_row(
    operation: &str,
    records: usize,
    items: usize,
    elapsed: Duration,
    result_count: usize,
    verified: bool,
    status: &str,
) {
    let total_ms = elapsed.as_secs_f64() * 1_000.0;
    let items_per_sec = if total_ms > 0.0 {
        items as f64 / (total_ms / 1_000.0)
    } else {
        0.0
    };
    println!(
        "{operation},{records},{items},{total_ms:.3},{items_per_sec:.0},{result_count},{verified},{status}"
    );
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok()
}

fn db_path() -> String {
    if let Ok(path) = std::env::var("PROLLY_SLATEDB_OPS_PATH") {
        return path.trim().trim_matches('/').to_string();
    }

    let prefix = std::env::var("PROLLY_SLATEDB_PATH_PREFIX")
        .unwrap_or_else(|_| "crabdb/prolly-bench".to_string());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "{}/slatedb-ops-{}-{nanos}",
        prefix.trim().trim_matches('/'),
        std::process::id()
    )
}

fn open_store(path: &str, object_store: Arc<dyn ObjectStore>) -> SlateDbStore {
    SlateDbStore::open(path.to_string(), object_store).unwrap()
}

fn build_slatedb_object_store() -> Arc<dyn ObjectStore> {
    Arc::new(
        AmazonS3Builder::new()
            .with_endpoint(slatedb_endpoint().trim_end_matches('/'))
            .with_bucket_name(slatedb_bucket().trim())
            .with_region(slatedb_region().trim())
            .with_access_key_id(slatedb_access_key_id())
            .with_secret_access_key(slatedb_secret_access_key())
            .with_allow_http(env_bool("PROLLY_SLATEDB_ALLOW_HTTP").unwrap_or(true))
            .with_virtual_hosted_style_request(false)
            .build()
            .unwrap(),
    )
}

fn env_bool(name: &str) -> Option<bool> {
    match std::env::var(name).ok()?.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn slatedb_endpoint() -> String {
    std::env::var("PROLLY_SLATEDB_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".to_string())
}

fn slatedb_bucket() -> String {
    std::env::var("PROLLY_SLATEDB_BUCKET").unwrap_or_else(|_| "crab".to_string())
}

fn slatedb_region() -> String {
    std::env::var("PROLLY_SLATEDB_REGION").unwrap_or_else(|_| "us-east-1".to_string())
}

fn slatedb_access_key_id() -> String {
    std::env::var("PROLLY_SLATEDB_ACCESS_KEY_ID").unwrap_or_else(|_| "crab".to_string())
}

fn slatedb_secret_access_key() -> String {
    std::env::var("PROLLY_SLATEDB_SECRET_ACCESS_KEY").unwrap_or_else(|_| "crab".to_string())
}

fn base_mutations(start: usize, count: usize) -> Vec<Mutation> {
    (start..start + count)
        .map(|i| Mutation::Upsert {
            key: key_for_index(i),
            val: value_for_index(i),
        })
        .collect()
}

fn update_mutations(records: usize, count: usize, label: &str) -> Vec<Mutation> {
    (0..count)
        .map(|i| Mutation::Upsert {
            key: key_for_index(i * 7 % records),
            val: format!("{label}-{i:012}").into_bytes(),
        })
        .collect()
}

fn update_mutations_for_indices(indices: &[usize], label: &str) -> Vec<Mutation> {
    indices
        .iter()
        .enumerate()
        .map(|(i, &idx)| Mutation::Upsert {
            key: key_for_index(idx),
            val: format!("{label}-{i:012}").into_bytes(),
        })
        .collect()
}

fn delete_mutations(records: usize, count: usize) -> Vec<Mutation> {
    (0..count)
        .map(|i| Mutation::Delete {
            key: key_for_index(i * 7 % records),
        })
        .collect()
}

fn delete_mutations_for_indices(indices: &[usize]) -> Vec<Mutation> {
    indices
        .iter()
        .map(|&idx| Mutation::Delete {
            key: key_for_index(idx),
        })
        .collect()
}

fn mixed_mutations(records: usize, count: usize, label: &str) -> Vec<Mutation> {
    (0..count)
        .map(|i| match i % 5 {
            0 => Mutation::Delete {
                key: key_for_index(i * 11 % records),
            },
            1 | 2 => Mutation::Upsert {
                key: key_for_index(i * 7 % records),
                val: format!("{label}-update-{i:012}").into_bytes(),
            },
            _ => Mutation::Upsert {
                key: format!("key-new-{label}-{i:012}").into_bytes(),
                val: format!("{label}-insert-{i:012}").into_bytes(),
            },
        })
        .collect()
}

fn mixed_mutations_for_indices(indices: &[usize], label: &str) -> Vec<Mutation> {
    indices
        .iter()
        .enumerate()
        .map(|(i, &idx)| match i % 5 {
            0 => Mutation::Delete {
                key: key_for_index(idx),
            },
            1 | 2 => Mutation::Upsert {
                key: key_for_index(idx),
                val: format!("{label}-update-{i:012}").into_bytes(),
            },
            _ => Mutation::Upsert {
                key: format!("key-new-{label}-{i:012}").into_bytes(),
                val: format!("{label}-insert-{i:012}").into_bytes(),
            },
        })
        .collect()
}

fn append_mutations(records: usize, count: usize, label: &str) -> Vec<Mutation> {
    (0..count)
        .map(|i| Mutation::Upsert {
            key: key_for_index(records + i),
            val: format!("{label}-{i:012}").into_bytes(),
        })
        .collect()
}

fn conflict_mutations(count: usize, label: &str) -> Vec<Mutation> {
    (0..count)
        .map(|i| Mutation::Upsert {
            key: key_for_index(i),
            val: format!("{label}-{i:012}").into_bytes(),
        })
        .collect()
}

fn range_changed_mutations(start_idx: usize, end_idx: usize, label: &str) -> Vec<Mutation> {
    (start_idx..end_idx)
        .enumerate()
        .filter_map(|(offset, i)| {
            let key = key_for_index(i);
            if offset % 7 == 0 {
                Some(Mutation::Delete { key })
            } else if offset % 3 == 0 {
                Some(Mutation::Upsert {
                    key,
                    val: format!("{label}-{i:012}").into_bytes(),
                })
            } else {
                None
            }
        })
        .collect()
}

fn verify_base_samples<S: prolly::Store>(prolly: &Prolly<S>, tree: &Tree, records: usize) -> bool {
    sample_indices(records).into_iter().all(|idx| {
        prolly.get(tree, &key_for_index(idx)).unwrap().as_deref()
            == Some(value_for_index(idx).as_slice())
    })
}

fn verify_updates<S: prolly::Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    count: usize,
    label: &str,
) -> bool {
    sample_indices(count).into_iter().all(|i| {
        prolly.get(tree, &key_for_index(i * 7)).unwrap().as_deref()
            == Some(format!("{label}-{i:012}").as_bytes())
    })
}

fn verify_updates_for_indices<S: prolly::Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    indices: &[usize],
    label: &str,
) -> bool {
    sample_indices(indices.len()).into_iter().all(|i| {
        prolly
            .get(tree, &key_for_index(indices[i]))
            .unwrap()
            .as_deref()
            == Some(format!("{label}-{i:012}").as_bytes())
    })
}

fn verify_deletes<S: prolly::Store>(prolly: &Prolly<S>, tree: &Tree, count: usize) -> bool {
    sample_indices(count)
        .into_iter()
        .all(|i| prolly.get(tree, &key_for_index(i * 7)).unwrap().is_none())
}

fn verify_deletes_for_indices<S: prolly::Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    indices: &[usize],
) -> bool {
    sample_indices(indices.len()).into_iter().all(|i| {
        prolly
            .get(tree, &key_for_index(indices[i]))
            .unwrap()
            .is_none()
    })
}

fn verify_mixed<S: prolly::Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    records: usize,
    count: usize,
    label: &str,
) -> bool {
    let insert_idx = (0..count).find(|i| i % 5 >= 3).unwrap();
    let update_idx = (0..count).find(|i| i % 5 == 1).unwrap();
    let delete_idx = (0..count).find(|i| i % 5 == 0).unwrap();
    prolly
        .get(tree, format!("key-new-{label}-{insert_idx:012}").as_bytes())
        .unwrap()
        .as_deref()
        == Some(format!("{label}-insert-{insert_idx:012}").as_bytes())
        && prolly
            .get(tree, &key_for_index(update_idx * 7 % records))
            .unwrap()
            .as_deref()
            == Some(format!("{label}-update-{update_idx:012}").as_bytes())
        && prolly
            .get(tree, &key_for_index(delete_idx * 11 % records))
            .unwrap()
            .is_none()
}

fn verify_mixed_for_indices<S: prolly::Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    indices: &[usize],
    label: &str,
) -> bool {
    let insert_idx = (0..indices.len()).find(|i| i % 5 >= 3).unwrap();
    let update_idx = (0..indices.len()).find(|i| i % 5 == 1).unwrap();
    let delete_idx = (0..indices.len()).find(|i| i % 5 == 0).unwrap();
    prolly
        .get(tree, format!("key-new-{label}-{insert_idx:012}").as_bytes())
        .unwrap()
        .as_deref()
        == Some(format!("{label}-insert-{insert_idx:012}").as_bytes())
        && prolly
            .get(tree, &key_for_index(indices[update_idx]))
            .unwrap()
            .as_deref()
            == Some(format!("{label}-update-{update_idx:012}").as_bytes())
        && prolly
            .get(tree, &key_for_index(indices[delete_idx]))
            .unwrap()
            .is_none()
}

fn verify_appends<S: prolly::Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    records: usize,
    count: usize,
    label: &str,
) -> bool {
    sample_indices(count).into_iter().all(|i| {
        prolly
            .get(tree, &key_for_index(records + i))
            .unwrap()
            .as_deref()
            == Some(format!("{label}-{i:012}").as_bytes())
    })
}

fn verify_conflict_resolution<S: prolly::Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    count: usize,
) -> bool {
    sample_indices(count).into_iter().all(|i| {
        prolly.get(tree, &key_for_index(i)).unwrap().as_deref()
            == Some(format!("right-conflict-{i:012}").as_bytes())
    })
}

fn sample_indices(len: usize) -> Vec<usize> {
    if len == 0 {
        return Vec::new();
    }
    let mut indices = vec![0, len / 2, len - 1];
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn random_indices(records: usize, count: usize, salt: u64) -> Vec<usize> {
    random_indices_in_range(0, records, count, salt)
}

fn random_indices_in_range(start: usize, end: usize, count: usize, salt: u64) -> Vec<usize> {
    let len = end.saturating_sub(start);
    if len == 0 || count == 0 {
        return Vec::new();
    }

    let count = count.min(len);
    let mut state = salt;
    let mut seen = HashSet::with_capacity(count);
    let mut indices = Vec::with_capacity(count);
    while indices.len() < count {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let idx = start + (mix64(state) as usize % len);
        if seen.insert(idx) {
            indices.push(idx);
        }
    }
    indices
}

fn spread_indices_in_range(start: usize, end: usize, count: usize, stride: usize) -> Vec<usize> {
    let len = end.saturating_sub(start);
    if len == 0 || count == 0 {
        return Vec::new();
    }

    let count = count.min(len);
    let stride = stride.max(1);
    let mut seen = HashSet::with_capacity(count);
    let mut indices = Vec::with_capacity(count);
    for i in 0..len {
        let idx = start + (i * stride % len);
        if seen.insert(idx) {
            indices.push(idx);
            if indices.len() == count {
                return indices;
            }
        }
    }
    for i in 0..len {
        let idx = start + i;
        if seen.insert(idx) {
            indices.push(idx);
            if indices.len() == count {
                break;
            }
        }
    }
    indices
}

fn clustered_indices(records: usize, count: usize, clusters: usize, salt: u64) -> Vec<usize> {
    clustered_indices_in_range(0, records, count, clusters, salt)
}

fn clustered_indices_in_range(
    start: usize,
    end: usize,
    count: usize,
    clusters: usize,
    salt: u64,
) -> Vec<usize> {
    let len = end.saturating_sub(start);
    if len == 0 || count == 0 {
        return Vec::new();
    }

    let count = count.min(len);
    let clusters = clusters.max(1).min(count);
    let per_cluster = count.div_ceil(clusters);
    let span = (per_cluster * 2).min(len).max(1);
    let mut seen = HashSet::with_capacity(count);
    let mut indices = Vec::with_capacity(count);

    for cluster in 0..clusters {
        let max_anchor = len.saturating_sub(span);
        let anchor = if max_anchor == 0 {
            0
        } else {
            mix64(salt.wrapping_add(cluster as u64)) as usize % (max_anchor + 1)
        };
        for offset in 0..span {
            let idx = start + anchor + offset;
            if seen.insert(idx) {
                indices.push(idx);
                if indices.len() == count {
                    return indices;
                }
            }
        }
    }

    let mut fill = random_indices_in_range(start, end, count, salt ^ 0xa5a5_a5a5_a5a5_a5a5);
    fill.retain(|idx| seen.insert(*idx));
    indices.extend(fill.into_iter().take(count - indices.len()));

    if indices.len() < count {
        for idx in start..end {
            if seen.insert(idx) {
                indices.push(idx);
                if indices.len() == count {
                    break;
                }
            }
        }
    }

    indices
}

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn key_for_index(i: usize) -> Vec<u8> {
    format!("key-{i:012}").into_bytes()
}

fn value_for_index(i: usize) -> Vec<u8> {
    format!("value-{i:012}-payload").into_bytes()
}

fn object_stats(object_store: Arc<dyn ObjectStore>, path: &str) -> ObjectStats {
    let runtime = runtime();
    let prefix = ObjectPath::from(path);
    runtime.block_on(async move {
        let mut stats = ObjectStats::default();
        let mut list = object_store.list(Some(&prefix));
        while let Some(meta) = list.next().await.transpose().unwrap() {
            stats.count += 1;
            stats.bytes += meta.size;
        }
        stats
    })
}

fn remove_slatedb_prefix(object_store: Arc<dyn ObjectStore>, path: &str) {
    let runtime = runtime();
    let prefix = ObjectPath::from(path);
    runtime.block_on(async move {
        let mut locations = Vec::new();
        let mut list = object_store.list(Some(&prefix));
        while let Some(meta) = list.next().await.transpose().unwrap() {
            locations.push(meta.location);
        }
        drop(list);
        for location in locations {
            let _ = object_store.delete(&location).await;
        }
    });
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .thread_name("prolly-slatedb-ops-bench")
        .enable_all()
        .build()
        .unwrap()
}

fn bench_config() -> Config {
    Config::builder()
        .min_chunk_size(64)
        .max_chunk_size(512)
        .chunking_factor(256)
        .hash_seed(0xC0DA)
        .build()
}
