use std::collections::BTreeMap;
use std::fs;

use rusqlite::{params, Connection};
use trail::{
    InitImportMode, LaneRetirementKind, LaneRetirementPhase, LaneRetirementProvenance,
    LaneRetirementReport, LaneWorkdirMode, Trail, WorkspaceLayerKeyV1,
};

#[test]
fn retirement_report_serializes_stable_kind_phase_and_compact_provenance() {
    let report = LaneRetirementReport {
        retirement_id: "ret_01".into(),
        lane_id: "lane_01".into(),
        former_name: "worker".into(),
        kind: LaneRetirementKind::Remove,
        phase: LaneRetirementPhase::BindingsRetired,
        resume_phase: Some(LaneRetirementPhase::BindingsRetired),
        forced: true,
        provenance: LaneRetirementProvenance {
            ref_name: "refs/lanes/worker".into(),
            base_change: "change_base".into(),
            head_change: "change_head".into(),
            base_root: "root_base".into(),
            head_root: "root_head".into(),
            view_id: Some("view_01".into()),
            environment_generation_ids: vec!["env_01".into()],
            source_bytes: 12,
            generated_bytes: 34,
            scratch_bytes: 56,
        },
        private_paths: vec!["/workspace/.trail/views/view_01/generated".into()],
        last_error_code: None,
        last_error_message: None,
        repair_command: Some("trail lane rm worker --force".into()),
        created_at: 10,
        updated_at: 11,
        completed_at: None,
    };

    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["kind"], "remove");
    assert_eq!(value["phase"], "bindings_retired");
    assert_eq!(
        value["provenance"]["environment_generation_ids"][0],
        "env_01"
    );
    assert_eq!(value["provenance"]["generated_bytes"], 34);
}

#[test]
fn fresh_workspace_has_no_lane_retirement_for_unknown_lane() {
    let root = tempfile::tempdir().unwrap();
    Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
    let db = Trail::open(root.path()).unwrap();
    assert!(db.lane_retirement("missing").unwrap().is_none());
}

#[test]
fn archive_is_reversible_and_preserves_lane_identity() {
    let root = tempfile::tempdir().unwrap();
    Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "archivable",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();

    let archived = db.archive_lane("archivable").unwrap();
    assert_eq!(archived.branch.status, "archived");
    assert_eq!(archived.record.lane_id, spawned.lane_id);
    assert_eq!(archived.branch.ref_name, spawned.ref_name);
    assert!(db.lane_initialization("archivable").unwrap().is_some());

    let restored = db.unarchive_lane("archivable").unwrap();
    assert_eq!(restored.branch.status, "active");
    assert_eq!(restored.record.lane_id, spawned.lane_id);
    assert_eq!(restored.branch.ref_name, spawned.ref_name);
}

#[test]
fn archive_http_routes_are_reversible() {
    let root = tempfile::tempdir().unwrap();
    Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "http-archive",
        Some("main"),
        LaneWorkdirMode::Virtual,
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    let archived = trail::server::handle_http_request(
        &mut db,
        b"POST /v1/lanes/http-archive/archive HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
    );
    assert_eq!(archived.status, 200);
    let archived: serde_json::Value = archived.body_json().unwrap();
    assert_eq!(archived["branch"]["status"], "archived");

    let restored = trail::server::handle_http_request(
        &mut db,
        b"POST /v1/lanes/http-archive/unarchive HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
    );
    assert_eq!(restored.status, 200);
    let restored: serde_json::Value = restored.body_json().unwrap();
    assert_eq!(restored["branch"]["status"], "active");
}

#[test]
fn archived_lane_is_not_execution_eligible() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "archived-exec",
        Some("main"),
        layered_mode(),
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();
    db.archive_lane("archived-exec").unwrap();

    let error = db
        .exec_lane_workspace("archived-exec", &["true".into()])
        .unwrap_err();
    assert!(error.to_string().contains("archived"), "{error}");
}

#[test]
fn completed_removal_discards_private_uppers_and_generation_bindings() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "disposable",
            Some("main"),
            layered_mode(),
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    let view = db.lane_workspace_view("disposable").unwrap().unwrap();
    for (directory, leaf) in [
        (&view.source_upper, "source.txt"),
        (&view.generated_upper, "build.bin"),
        (&view.scratch_upper, "scratch.tmp"),
    ] {
        fs::create_dir_all(directory).unwrap();
        fs::write(std::path::Path::new(directory).join(leaf), b"disposable").unwrap();
    }

    let layer_dir = root.path().join(".trail/cache/layers/test-removal-layer");
    fs::create_dir_all(&layer_dir).unwrap();
    fs::write(layer_dir.join("artifact"), b"shared").unwrap();
    let sqlite = root.path().join(".trail/index/trail.sqlite");
    let conn = Connection::open(&sqlite).unwrap();
    conn.execute(
        "INSERT INTO workspace_layers(
             layer_id,kind,cache_key,adapter,adapter_version,storage_path,state,
             logical_bytes,physical_bytes,entry_count,portability_scope,
             builder_id,lease_expires_at,last_used_at,created_at)
         VALUES('layer_removal','dependency','cache_removal','test',1,?1,'ready',
                6,6,1,'platform',NULL,NULL,1,1)",
        [layer_dir.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO workspace_view_layers(
             view_id,layer_id,mount_path,priority,read_only,source_path)
         VALUES(?1,'layer_removal','vendor',100,1,'')",
        [&view.view_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO environment_generations(
             generation_id,view_id,generation_sequence,source_root,specification_digest,
             predecessor_generation_id,state,created_at,activated_at,retired_at)
         VALUES('env_removal',?1,1,?2,'spec',NULL,'active',1,1,NULL)",
        params![&view.view_id, view.base_root.0],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO environment_generation_components(
             generation_id,component_id,adapter_identity,kind,component_key,layer_id,mount_path)
         VALUES('env_removal','dependency','trail/test@1','dependency','key',
                'layer_removal','vendor')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO environment_view_generations(view_id,generation_id,updated_at)
         VALUES(?1,'env_removal',1)",
        [&view.view_id],
    )
    .unwrap();
    drop(conn);

    db.remove_lane("disposable", true).unwrap();

    for path in [
        &view.source_upper,
        &view.generated_upper,
        &view.scratch_upper,
        &view.meta_dir,
    ] {
        assert!(
            !std::path::Path::new(path).exists(),
            "{path} survived removal"
        );
    }
    let conn = Connection::open(sqlite).unwrap();
    for (table, predicate) in [
        ("workspace_views", "lane_id='lane_placeholder'"),
        ("workspace_view_layers", "view_id='view_placeholder'"),
        ("environment_view_generations", "view_id='view_placeholder'"),
        ("environment_generations", "view_id='view_placeholder'"),
    ] {
        let predicate = predicate
            .replace("lane_placeholder", &spawned.lane_id)
            .replace("view_placeholder", &view.view_id);
        let count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table} retained disposable state");
    }
    let retirement = db.lane_retirement(&spawned.lane_id).unwrap().unwrap();
    assert_eq!(retirement.phase, LaneRetirementPhase::Completed);
    assert_eq!(
        retirement.provenance.environment_generation_ids,
        vec!["env_removal"]
    );
}

#[test]
fn open_recovers_an_interrupted_binding_retirement_to_completion() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "recover-removal",
            Some("main"),
            layered_mode(),
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    let details = db.lane_details("recover-removal").unwrap();
    let view = db.lane_workspace_view("recover-removal").unwrap().unwrap();
    fs::create_dir_all(&view.generated_upper).unwrap();
    let artifact = std::path::Path::new(&view.generated_upper).join("partial.bin");
    fs::write(&artifact, b"partial").unwrap();
    let view_root = std::path::Path::new(&view.meta_dir)
        .parent()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let provenance = LaneRetirementProvenance {
        ref_name: details.branch.ref_name.clone(),
        base_change: details.branch.base_change.0.clone(),
        head_change: details.branch.head_change.0.clone(),
        base_root: details.branch.base_root.0.clone(),
        head_root: details.branch.head_root.0.clone(),
        view_id: Some(view.view_id.clone()),
        environment_generation_ids: Vec::new(),
        source_bytes: 0,
        generated_bytes: 7,
        scratch_bytes: 0,
    };
    let private_paths = vec![
        view.source_upper.clone(),
        view.generated_upper.clone(),
        view.scratch_upper.clone(),
        view_root,
        details.branch.workdir.clone().unwrap(),
    ];
    let sqlite = root.path().join(".trail/index/trail.sqlite");
    let conn = Connection::open(&sqlite).unwrap();
    conn.execute(
        "INSERT INTO lane_retirements(
             retirement_id,lane_id,former_name,kind,phase,forced,
             provenance_json,private_paths_json,repair_command,created_at,updated_at)
         VALUES('ret_interrupted',?1,'recover-removal','remove','bindings_retired',1,
                ?2,?3,'trail lane rm recover-removal --force',1,1)",
        params![
            &spawned.lane_id,
            serde_json::to_vec(&provenance).unwrap(),
            serde_json::to_vec(&private_paths).unwrap()
        ],
    )
    .unwrap();
    conn.execute(
        "UPDATE workspace_views SET status='retiring' WHERE view_id=?1",
        [&view.view_id],
    )
    .unwrap();
    drop(conn);
    drop(db);

    let recovered = Trail::open(root.path()).unwrap();
    let retirement = recovered
        .lane_retirement(&spawned.lane_id)
        .unwrap()
        .unwrap();
    assert_eq!(retirement.phase, LaneRetirementPhase::Completed);
    assert!(!artifact.exists());
    assert_eq!(
        recovered
            .lane_details(&spawned.lane_id)
            .unwrap()
            .branch
            .status,
        "removed"
    );
}

#[test]
fn purge_requires_force_and_exact_lane_id_then_erases_tombstone() {
    let root = tempfile::tempdir().unwrap();
    Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "purge-me",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    let session = db
        .start_lane_session("purge-me", Some("purge provenance".into()), None)
        .unwrap()
        .session;
    let turn = db
        .begin_lane_session_turn("purge-me", &session.session_id, None)
        .unwrap()
        .turn;
    db.add_lane_turn_message(&turn.turn_id, "user", "purge all lane provenance")
        .unwrap();
    db.end_lane_turn(&turn.turn_id, "completed").unwrap();
    db.end_lane_session(&session.session_id, "completed")
        .unwrap();
    db.remove_lane("purge-me", true).unwrap();

    let no_force = db.purge_lane(&spawned.lane_id, false).unwrap_err();
    assert!(no_force.to_string().contains("--force"), "{no_force}");
    let former_name = db.purge_lane("purge-me", true).unwrap_err();
    assert!(
        former_name.to_string().contains("exact lane ID"),
        "{former_name}"
    );

    let purged = db.purge_lane(&spawned.lane_id, true).unwrap();
    assert_eq!(purged.kind, LaneRetirementKind::Purge);
    assert_eq!(purged.phase, LaneRetirementPhase::Completed);
    assert!(db.lane_retirement(&spawned.lane_id).unwrap().is_none());
    assert!(db.lane_details(&spawned.lane_id).is_err());
    let conn = Connection::open(root.path().join(".trail/index/trail.sqlite")).unwrap();
    for table in ["lane_sessions", "lane_turns", "lane_events", "messages"] {
        let count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE lane_id=?1"),
                [&spawned.lane_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table} retained purged lane provenance");
    }
}

#[test]
fn removal_makes_unique_layers_collectable_but_preserves_shared_layers() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "root\n").unwrap();
    Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    for lane in ["layer-a", "layer-b"] {
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            layered_mode(),
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    }
    let built = tempfile::tempdir().unwrap();
    fs::write(built.path().join("artifact"), b"immutable").unwrap();
    let key = WorkspaceLayerKeyV1 {
        kind: "dependency".into(),
        adapter: "test".into(),
        adapter_version: 1,
        inputs: BTreeMap::from([("lock".into(), "digest".into())]),
        tool_versions: BTreeMap::from([("tool".into(), "1".into())]),
        platform: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        portability_scope: "platform".into(),
        strategy: "test".into(),
    };
    let layer = db
        .publish_workspace_layer_from_directory(&key, built.path())
        .unwrap();
    for lane in ["layer-a", "layer-b"] {
        db.attach_workspace_layer(lane, &layer.layer_id, "vendor", "test", &layer.cache_key)
            .unwrap();
    }

    db.remove_lane("layer-a", true).unwrap();
    let shared = db.workspace_cache_gc(true, Some(0)).unwrap();
    assert!(!shared
        .candidates
        .iter()
        .any(|candidate| candidate.id == layer.layer_id));

    db.remove_lane("layer-b", true).unwrap();
    let unique = db.workspace_cache_gc(true, Some(0)).unwrap();
    assert!(unique
        .candidates
        .iter()
        .any(|candidate| candidate.id == layer.layer_id));
}

#[test]
fn cleanup_failure_records_repair_phase_and_resumes_from_exact_cut() {
    let root = tempfile::tempdir().unwrap();
    Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
    let mut db = Trail::open(root.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "repair-removal",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    let details = db.lane_details("repair-removal").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let protected = outside.path().join("must-survive");
    fs::write(&protected, b"foreign").unwrap();
    let provenance = LaneRetirementProvenance {
        ref_name: details.branch.ref_name.clone(),
        base_change: details.branch.base_change.0.clone(),
        head_change: details.branch.head_change.0.clone(),
        base_root: details.branch.base_root.0.clone(),
        head_root: details.branch.head_root.0.clone(),
        view_id: None,
        environment_generation_ids: Vec::new(),
        source_bytes: 0,
        generated_bytes: 0,
        scratch_bytes: 0,
    };
    let sqlite = root.path().join(".trail/index/trail.sqlite");
    Connection::open(&sqlite)
        .unwrap()
        .execute(
            "INSERT INTO lane_retirements(
                 retirement_id,lane_id,former_name,kind,phase,forced,
                 provenance_json,private_paths_json,repair_command,created_at,updated_at)
             VALUES('ret_repair',?1,'repair-removal','remove','bindings_retired',1,
                    ?2,?3,'trail lane rm repair-removal --force',1,1)",
            params![
                &spawned.lane_id,
                serde_json::to_vec(&provenance).unwrap(),
                serde_json::to_vec(&vec![protected.to_string_lossy().into_owned()]).unwrap()
            ],
        )
        .unwrap();

    let error = db.remove_lane("repair-removal", true).unwrap_err();
    assert!(error.to_string().contains("not confined"), "{error}");
    let repair = db.lane_retirement(&spawned.lane_id).unwrap().unwrap();
    assert_eq!(repair.phase, LaneRetirementPhase::RepairRequired);
    assert_eq!(
        repair.resume_phase,
        Some(LaneRetirementPhase::BindingsRetired)
    );
    assert!(repair.last_error_message.is_some());
    assert_eq!(fs::read(&protected).unwrap(), b"foreign");

    let respawn_error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "repair-removal",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap_err();
    assert!(
        matches!(
            respawn_error,
            trail::Error::OperationCommittedRepairRequired { .. }
        ),
        "incomplete retirement escaped as {respawn_error:?}"
    );
    let lane_count: i64 = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM lanes WHERE name='repair-removal'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(lane_count, 1, "respawn created a second active lane");

    Connection::open(&sqlite)
        .unwrap()
        .execute(
            "UPDATE lane_retirements SET private_paths_json=?1
             WHERE retirement_id='ret_repair'",
            [serde_json::to_vec(&Vec::<String>::new()).unwrap()],
        )
        .unwrap();
    let completed = db.resume_lane_retirement(&spawned.lane_id).unwrap();
    assert_eq!(completed.phase, LaneRetirementPhase::Completed);
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
