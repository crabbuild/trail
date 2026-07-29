use std::collections::BTreeMap;
use std::fs;

use rusqlite::{params, Connection};
use trail::{InitImportMode, LaneWorkdirMode, Trail, WorkspaceLayerKeyV1};

#[test]
fn lane_fork_inherits_verified_immutable_layer_with_fresh_private_uppers() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "parent",
        Some("main"),
        layered_mode(),
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();
    let parent_view = db.lane_workspace_view("parent").unwrap().unwrap();

    let built = tempfile::tempdir().unwrap();
    fs::create_dir_all(built.path().join("pkg")).unwrap();
    fs::write(built.path().join("pkg/index.js"), "module.exports = 1;\n").unwrap();
    let key = WorkspaceLayerKeyV1 {
        kind: "dependency".into(),
        adapter: "node".into(),
        adapter_version: 1,
        inputs: BTreeMap::from([("package-lock.json".into(), "lock-digest".into())]),
        tool_versions: BTreeMap::from([("node".into(), "22".into())]),
        platform: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        portability_scope: "platform".into(),
        strategy: "npm-ci-ignore-scripts".into(),
    };
    let layer = db
        .publish_workspace_layer_from_directory(&key, built.path())
        .unwrap();
    db.attach_workspace_layer(
        "parent",
        &layer.layer_id,
        "node_modules",
        "node",
        &layer.cache_key,
    )
    .unwrap();

    let sqlite = root.path().join(".trail/index/trail.sqlite");
    let conn = Connection::open(&sqlite).unwrap();
    conn.execute(
        "INSERT INTO environment_generations(
             generation_id,view_id,generation_sequence,source_root,specification_digest,
             predecessor_generation_id,state,created_at,activated_at,retired_at)
         VALUES('env_parent',?1,1,?2,'spec-parent',NULL,'active',1,1,NULL)",
        params![&parent_view.view_id, parent_view.base_root.0],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO environment_generation_components(
             generation_id,component_id,adapter_identity,kind,component_key,layer_id,mount_path)
         VALUES('env_parent','node','trail/node@1','dependency',?1,?2,'node_modules')",
        params![&layer.cache_key, &layer.layer_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO environment_view_generations(view_id,generation_id,updated_at)
         VALUES(?1,'env_parent',1)",
        [&parent_view.view_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO environment_generation_components(
             generation_id,component_id,adapter_identity,kind,component_key,layer_id,mount_path)
         VALUES('env_parent','corrupt-cache','trail/cache@1','dependency',
                'corrupt-key','layer_missing','vendor/cache')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO environment_generation_runtime_resources(
             generation_id,component_id,resource_name,runtime_type,provider,artifact_name,
             image_reference,image_digest,image_platform,container_port,protocol,
             health_type,health_timeout_ms,restart_policy,cleanup_owner,volume_target,
             allocation_id,provider_resource_id,container_name,network_name,volume_name,
             host_port,status,health_status,reason,cleanup_token,owner_pid,owner_start_token,
             created_at,updated_at,started_at,stopped_at)
         VALUES('env_parent','node','dev-server','oci','docker','image',
                'node:22','sha256:abc','linux/amd64',3000,'tcp',
                'tcp',1000,'no','trail',NULL,
                'allocation_parent','container_parent','trail-parent','trail-network',
                NULL,NULL,'running','healthy',NULL,'cleanup-parent',NULL,NULL,1,1,1,NULL)",
        [],
    )
    .unwrap();
    drop(conn);

    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "child",
        Some("parent"),
        layered_mode(),
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    let child_view = db.lane_workspace_view("child").unwrap().unwrap();
    assert_ne!(child_view.view_id, parent_view.view_id);
    assert_ne!(child_view.source_upper, parent_view.source_upper);
    assert_ne!(child_view.generated_upper, parent_view.generated_upper);
    assert_ne!(child_view.scratch_upper, parent_view.scratch_upper);
    let generation = db
        .active_environment_generation("child")
        .unwrap()
        .expect("child should inherit an active environment generation");
    assert_eq!(
        generation.predecessor_generation_id.as_deref(),
        Some("env_parent")
    );
    assert_eq!(generation.components.len(), 1);
    assert_eq!(
        generation.components[0].layer_id.as_deref(),
        Some(layer.layer_id.as_str())
    );

    let conn = Connection::open(sqlite).unwrap();
    let child_binding: String = conn
        .query_row(
            "SELECT layer_id FROM workspace_view_layers
             WHERE view_id=?1 AND mount_path='node_modules'",
            [&child_view.view_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(child_binding, layer.layer_id);
    let inherited_runtime_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM environment_generation_runtime_resources
             WHERE generation_id=?1",
            [&generation.generation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(inherited_runtime_count, 0);
    drop(conn);
    let inheritance_event = db
        .list_lane_events(
            Some("child"),
            None,
            None,
            Some("lane_environment_inheritance"),
            10,
        )
        .unwrap()
        .into_iter()
        .next()
        .expect("fork must report inherited and rejected components");
    let components = inheritance_event.payload.unwrap()["components"]
        .as_array()
        .unwrap()
        .clone();
    assert!(components.iter().any(|component| {
        component["component_id"] == "node" && component["status"] == "inherited"
    }));
    assert!(components.iter().any(|component| {
        component["component_id"] == "corrupt-cache"
            && component["status"] == "rejected"
            && component["reason"] == "layer_verification_failed"
    }));

    let fork_names = (0..4)
        .map(|index| format!("concurrent-child-{index}"))
        .collect::<Vec<_>>();
    let root_path = root.path();
    std::thread::scope(|scope| {
        let mut forks = Vec::new();
        for fork_name in &fork_names {
            let fork_name = fork_name.clone();
            forks.push(scope.spawn(move || {
                let mut fork_db = Trail::open(root_path).unwrap();
                fork_db
                    .spawn_lane_with_workdir_mode_paths_and_neighbors(
                        &fork_name,
                        Some("parent"),
                        layered_mode(),
                        None,
                        None,
                        None,
                        &[],
                        false,
                    )
                    .unwrap();
            }));
        }
        for fork in forks {
            fork.join().unwrap();
        }
    });
    for fork_name in &fork_names {
        let fork_generation = db
            .active_environment_generation(fork_name)
            .unwrap()
            .unwrap();
        assert_eq!(
            fork_generation.components[0].layer_id.as_deref(),
            Some(layer.layer_id.as_str())
        );
        let fork_view = db.lane_workspace_view(fork_name).unwrap().unwrap();
        assert_ne!(fork_view.source_upper, child_view.source_upper);
        assert_ne!(fork_view.generated_upper, child_view.generated_upper);
        assert_ne!(fork_view.scratch_upper, child_view.scratch_upper);
    }

    let conn = Connection::open(root.path().join(".trail/index/trail.sqlite")).unwrap();
    conn.execute(
        "DELETE FROM environment_generation_runtime_resources
         WHERE generation_id='env_parent'",
        [],
    )
    .unwrap();
    drop(conn);
    db.remove_lane("parent", true).unwrap();
    let child_after_parent_removal = db.active_environment_generation("child").unwrap().unwrap();
    assert_eq!(
        child_after_parent_removal.components[0].layer_id.as_deref(),
        Some(layer.layer_id.as_str())
    );
    for fork_name in &fork_names {
        let fork_generation = db
            .active_environment_generation(fork_name)
            .unwrap()
            .unwrap();
        assert_eq!(
            fork_generation.components[0].layer_id.as_deref(),
            Some(layer.layer_id.as_str())
        );
    }
    db.verify_workspace_layer(&layer.layer_id).unwrap();
}

#[test]
fn fork_without_parent_generation_is_safe_and_reports_why_reuse_was_skipped() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "empty-parent",
        Some("main"),
        layered_mode(),
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "empty-child",
        Some("empty-parent"),
        layered_mode(),
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    assert!(db
        .active_environment_generation("empty-child")
        .unwrap()
        .is_none());
    let event = db
        .list_lane_events(
            Some("empty-child"),
            None,
            None,
            Some("lane_environment_inheritance"),
            10,
        )
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        event.payload.unwrap()["reason"],
        "parent_has_no_active_generation"
    );
}

fn layered_mode() -> LaneWorkdirMode {
    if cfg!(target_os = "macos") {
        LaneWorkdirMode::NfsCow
    } else if cfg!(target_os = "windows") {
        LaneWorkdirMode::DokanCow
    } else {
        LaneWorkdirMode::FuseCow
    }
}
