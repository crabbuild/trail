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
        "INSERT INTO environment_component_states(
             view_id,component_id,adapter_identity,adapter_version,implementation_version,
             distribution_digest,kind,expected_key,attached_key,status,reason,updated_at)
         VALUES(?1,'node','trail/node@1',1,?2,'builtin:node-plan-v1','dependency',
                ?3,?3,'ready',NULL,1)",
        params![
            &parent_view.view_id,
            env!("CARGO_PKG_VERSION"),
            &layer.cache_key
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO environment_generation_outputs(
             generation_id,component_id,output_name,policy,reuse_mode,sharing_scope,
             publication_trigger,publication_gate,storage_identity,layer_id,
             manifest_object_id,publication_id,mount_path,layer_subpath)
         SELECT 'env_parent','node','dependencies','immutable_seed_private','exact','workspace',
                'on_sync',NULL,?1,?2,manifest_object_id,NULL,'node_modules',''
         FROM workspace_layers WHERE layer_id=?2",
        params![&layer.cache_key, &layer.layer_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO environment_generation_outputs(
             generation_id,component_id,output_name,policy,reuse_mode,sharing_scope,
             publication_trigger,publication_gate,storage_identity,layer_id,
             manifest_object_id,publication_id,mount_path,layer_subpath)
         VALUES('env_parent','node','rejected-sibling','immutable_seed_private','exact',
                'workspace','on_sync',NULL,'rejected-key','layer_missing','object_missing',NULL,
                'vendor/rejected','')",
        [],
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
        "INSERT INTO environment_generation_outputs(
             generation_id,component_id,output_name,policy,reuse_mode,sharing_scope,
             publication_trigger,publication_gate,storage_identity,layer_id,
             manifest_object_id,publication_id,mount_path,layer_subpath)
         VALUES('env_parent','corrupt-cache','dependencies','immutable_seed_private','exact',
                'workspace','on_sync',NULL,'corrupt-key','layer_missing','object_missing',NULL,
                'vendor/cache','')",
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

    let conn = Connection::open(&sqlite).unwrap();
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
    let child_output_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM environment_generation_outputs WHERE generation_id=?1",
            [&generation.generation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(child_output_count, 1);
    let child_artifact_binding: (String, String, String, String) = conn
        .query_row(
            "SELECT desired_key,envelope_id,tree_root_id,binding_identity
             FROM artifact_generation_bindings
             WHERE generation_id=?1 AND component_id='node' AND output_name='dependencies'",
            [&generation.generation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(child_artifact_binding.0, layer.cache_key);
    assert!(child_artifact_binding.1.starts_with("artifact_envelope_"));
    assert!(child_artifact_binding.2.starts_with("artifact_tree_"));
    assert!(child_artifact_binding.3.starts_with("artifact_binding_"));
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
    let outputs = inheritance_event.payload.unwrap()["outputs"]
        .as_array()
        .unwrap()
        .clone();
    assert!(outputs
        .iter()
        .any(|output| { output["component_id"] == "node" && output["decision"] == "reused" }));
    assert!(outputs.iter().any(|output| {
        output["component_id"] == "corrupt-cache"
            && output["decision"] == "rejected"
            && output["reason"] == "layer_verification_failed"
    }));
    assert!(outputs.iter().any(|output| {
        output["component_id"] == "node"
            && output["output_name"] == "rejected-sibling"
            && output["decision"] == "rejected"
            && output["reason"] == "layer_verification_failed"
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

    let conn = Connection::open(&sqlite).unwrap();
    let mut artifact_binding_identities = conn
        .prepare(
            "SELECT b.binding_identity
             FROM artifact_generation_bindings b
             JOIN environment_generations g ON g.generation_id=b.generation_id
             WHERE b.component_id='node' AND b.output_name='dependencies'
             ORDER BY b.binding_identity",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    artifact_binding_identities.dedup();
    assert_eq!(artifact_binding_identities.len(), fork_names.len() + 1);
    drop(conn);

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
