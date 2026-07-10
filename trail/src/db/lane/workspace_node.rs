use super::*;

#[derive(Clone, Debug)]
struct NodeEnvironmentInputs {
    package_root: String,
    manager: String,
    files: BTreeMap<String, FileEntry>,
    key: WorkspaceLayerKeyV1,
}

impl Trail {
    pub fn sync_node_dependencies(
        &self,
        lane: &str,
        package_root: Option<&str>,
    ) -> Result<WorkspaceLayerReport> {
        let branch = self.lane_branch(lane)?;
        let head = self.get_ref(&branch.ref_name)?;
        let inputs = self.node_environment_inputs(&head.root_id, package_root.unwrap_or(""))?;
        let cache_key = self.workspace_layer_cache_key(&inputs.key)?;
        let adapter_id = if inputs.package_root.is_empty() {
            "node".to_string()
        } else {
            format!("node:{}", inputs.package_root)
        };
        self.set_workspace_environment_state(
            lane,
            &adapter_id,
            &cache_key,
            None,
            "building",
            None,
        )?;
        let build = self.build_workspace_layer_singleflight(&inputs.key, |build_dir| {
            let project = build_dir.join("project");
            fs::create_dir_all(&project)?;
            for (path, entry) in &inputs.files {
                let relative = strip_package_root(path, &inputs.package_root)?;
                let destination = safe_join(&project, &relative)?;
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                let projection = self.project_entry_file(entry)?;
                fs::copy(projection, &destination)?;
                let mut permissions = fs::metadata(&destination)?.permissions();
                permissions.set_readonly(false);
                fs::set_permissions(&destination, permissions)?;
            }
            let cache = self.db_dir.join("cache/tool-home/node");
            fs::create_dir_all(&cache)?;
            let mut command = Command::new(&inputs.manager);
            command
                .current_dir(&project)
                .env("npm_config_cache", cache.join("npm"))
                .env("PNPM_HOME", cache.join("pnpm-home"))
                .env("PNPM_STORE_DIR", cache.join("pnpm-store"));
            match inputs.manager.as_str() {
                "npm" => {
                    command.args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"]);
                }
                "pnpm" => {
                    command.args(["install", "--frozen-lockfile", "--ignore-scripts"]);
                }
                "yarn" => {
                    let version = inputs
                        .key
                        .tool_versions
                        .get("yarn")
                        .map(String::as_str)
                        .unwrap_or_default();
                    if version.starts_with('1') {
                        command.args(["install", "--frozen-lockfile", "--ignore-scripts"]);
                    } else {
                        command.args(["install", "--immutable", "--mode=skip-build"]);
                    }
                }
                "bun" => {
                    command.args(["install", "--frozen-lockfile", "--ignore-scripts"]);
                }
                other => {
                    return Err(Error::InvalidInput(format!(
                        "unsupported Node package manager `{other}`"
                    )))
                }
            }
            let status = command.status().map_err(|err| {
                Error::InvalidInput(format!(
                    "failed to launch `{}` for Node dependency layer: {err}",
                    inputs.manager
                ))
            })?;
            if !status.success() {
                return Err(Error::InvalidInput(format!(
                    "{} frozen install failed with {status}",
                    inputs.manager
                )));
            }
            let output = project.join("node_modules");
            fs::create_dir_all(&output)?;
            Ok(output)
        });
        let layer = match build {
            Ok(layer) => layer,
            Err(err) => {
                self.set_workspace_environment_state(
                    lane,
                    &adapter_id,
                    &cache_key,
                    None,
                    "failed",
                    Some(&err.to_string()),
                )?;
                return Err(err);
            }
        };
        let mount_path = if inputs.package_root.is_empty() {
            "node_modules".to_string()
        } else {
            format!("{}/node_modules", inputs.package_root)
        };
        self.attach_workspace_layer(lane, &layer.layer_id, &mount_path, &adapter_id, &cache_key)?;
        Ok(layer)
    }

    pub fn workspace_environment_status(
        &self,
        lane: &str,
    ) -> Result<Vec<WorkspaceEnvironmentReport>> {
        let branch = self.lane_branch(lane)?;
        let head = self.get_ref(&branch.ref_name)?;
        let mut reports = self.workspace_environment_rows(lane)?;
        for report in &mut reports {
            let package_root = if report.adapter == "node" {
                ""
            } else if let Some(root) = report.adapter.strip_prefix("node:") {
                root
            } else {
                continue;
            };
            let inputs = self.node_environment_inputs(&head.root_id, package_root)?;
            let expected = self.workspace_layer_cache_key(&inputs.key)?;
            if report.attached_key.as_deref() == Some(expected.as_str()) {
                report.expected_key = expected;
                report.status = "ready".to_string();
                report.reason = None;
            } else if report.expected_key != expected {
                report.expected_key = expected;
                report.status = "stale".to_string();
                report.reason = Some("package or lock inputs changed".to_string());
            }
        }
        Ok(reports)
    }

    fn workspace_environment_rows(&self, lane: &str) -> Result<Vec<WorkspaceEnvironmentReport>> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let mut stmt = self.conn.prepare(
            "SELECT view_id, adapter, expected_key, attached_key, status, reason, updated_at FROM workspace_environment_states WHERE view_id = ?1 ORDER BY adapter",
        )?;
        let reports = stmt
            .query_map(params![view.view_id], |row| {
                Ok(WorkspaceEnvironmentReport {
                    view_id: row.get(0)?,
                    adapter: row.get(1)?,
                    expected_key: row.get(2)?,
                    attached_key: row.get(3)?,
                    status: row.get(4)?,
                    reason: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(reports)
    }

    pub fn refresh_workspace_environment_staleness(&self, lane: &str) -> Result<()> {
        let states = self.workspace_environment_status(lane)?;
        for state in states {
            self.set_workspace_environment_state(
                lane,
                &state.adapter,
                &state.expected_key,
                state.attached_key.as_deref(),
                &state.status,
                state.reason.as_deref(),
            )?;
        }
        Ok(())
    }

    fn node_environment_inputs(
        &self,
        root_id: &ObjectId,
        package_root: &str,
    ) -> Result<NodeEnvironmentInputs> {
        let package_root = if package_root.trim_matches('/').is_empty() {
            String::new()
        } else {
            normalize_relative_path(package_root)?
        };
        let package_json = join_repo_path(&package_root, "package.json");
        let package_entry = self
            .root_file_entry(root_id, &package_json)?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "Node package root `{}` has no package.json",
                    if package_root.is_empty() {
                        "."
                    } else {
                        &package_root
                    }
                ))
            })?;
        let lock_names = [
            ("pnpm-lock.yaml", "pnpm"),
            ("yarn.lock", "yarn"),
            ("bun.lock", "bun"),
            ("bun.lockb", "bun"),
            ("npm-shrinkwrap.json", "npm"),
            ("package-lock.json", "npm"),
        ];
        let mut selected = None;
        for (name, manager) in lock_names {
            let path = join_repo_path(&package_root, name);
            if let Some(entry) = self.root_file_entry(root_id, &path)? {
                selected = Some((path, manager.to_string(), entry));
                break;
            }
        }
        let (lock_path, manager, lock_entry) = selected.ok_or_else(|| {
            Error::InvalidInput(format!(
                "Node package root `{}` has no supported frozen-install lockfile",
                if package_root.is_empty() {
                    "."
                } else {
                    &package_root
                }
            ))
        })?;
        let manager_version = tool_version(&manager)?;
        let node_version = tool_version("node")?;
        let mut files = BTreeMap::from([
            (package_json.clone(), package_entry.clone()),
            (lock_path.clone(), lock_entry.clone()),
        ]);
        // Workspace package manifests affect installation. Discover them in a
        // streaming pass; this runs only during explicit dependency sync, not
        // during view creation or lookup.
        self.for_each_root_file_chunk(root_id, 1024, |chunk| {
            for (path, entry) in chunk {
                if Path::new(&path).file_name().and_then(|name| name.to_str())
                    != Some("package.json")
                {
                    continue;
                }
                if package_root.is_empty()
                    || path.starts_with(&format!("{package_root}/"))
                    || path == package_json
                {
                    files.insert(path, entry);
                }
            }
            Ok(())
        })?;
        let key = WorkspaceLayerKeyV1 {
            kind: "dependency".to_string(),
            adapter: "node".to_string(),
            adapter_version: 1,
            inputs: files
                .iter()
                .map(|(path, entry)| (path.clone(), entry.content_hash.clone()))
                .collect(),
            tool_versions: BTreeMap::from([
                ("node".to_string(), node_version),
                (manager.clone(), manager_version),
            ]),
            platform: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            portability_scope: "platform-architecture-node-abi".to_string(),
            strategy: format!("{manager}-frozen-ignore-scripts-v1"),
        };
        Ok(NodeEnvironmentInputs {
            package_root,
            manager,
            files,
            key,
        })
    }

    fn set_workspace_environment_state(
        &self,
        lane: &str,
        adapter: &str,
        expected_key: &str,
        attached_key: Option<&str>,
        status: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        self.conn.execute(
            "INSERT OR REPLACE INTO workspace_environment_states (view_id, adapter, expected_key, attached_key, status, reason, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![view.view_id, adapter, expected_key, attached_key, status, reason, now_ts()],
        )?;
        Ok(())
    }
}

fn tool_version(tool: &str) -> Result<String> {
    let output = Command::new(tool)
        .arg("--version")
        .output()
        .map_err(|err| {
            Error::InvalidInput(format!("required tool `{tool}` is unavailable: {err}"))
        })?;
    if !output.status.success() {
        return Err(Error::InvalidInput(format!(
            "`{tool} --version` failed with {}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn join_repo_path(root: &str, name: &str) -> String {
    if root.is_empty() {
        name.to_string()
    } else {
        format!("{root}/{name}")
    }
}

fn strip_package_root(path: &str, package_root: &str) -> Result<String> {
    if package_root.is_empty() {
        return normalize_relative_path(path);
    }
    path.strip_prefix(&format!("{package_root}/"))
        .ok_or_else(|| Error::InvalidPath {
            path: path.to_string(),
            reason: format!("path is outside Node package root `{package_root}`"),
        })
        .and_then(normalize_relative_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_lanes_with_identical_node_inputs_reuse_one_real_frozen_install() {
        if Command::new("npm").arg("--version").output().is_err()
            || Command::new("node").arg("--version").output().is_err()
        {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("package.json"),
            r#"{"name":"trail-node-layer-test","version":"1.0.0","private":true}"#,
        )
        .unwrap();
        fs::write(
            workspace.path().join("package-lock.json"),
            r#"{"name":"trail-node-layer-test","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"trail-node-layer-test","version":"1.0.0"}}}"#,
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::OverlayCow
        };
        for lane in ["node-one", "node-two"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                mode.clone(),
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }
        let first = db.sync_node_dependencies("node-one", None).unwrap();
        let second = db.sync_node_dependencies("node-two", None).unwrap();
        assert_eq!(first.layer_id, second.layer_id);
        assert_eq!(first.cache_key, second.cache_key);
        assert_eq!(db.list_workspace_layers().unwrap().len(), 1);
        let view_one = db.lane_workspace_view("node-one").unwrap().unwrap();
        let view_two = db.lane_workspace_view("node-two").unwrap().unwrap();
        let bound = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM workspace_view_layers WHERE layer_id = ?1 AND view_id IN (?2, ?3)",
                params![first.layer_id, view_one.view_id, view_two.view_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(bound, 2);

        db.conn
            .execute(
                "UPDATE workspace_environment_states SET status = 'building', reason = 'sentinel', updated_at = -1 WHERE view_id = ?1",
                params![view_one.view_id],
            )
            .unwrap();
        let dynamic = db.workspace_environment_status("node-one").unwrap();
        assert_eq!(dynamic[0].status, "ready");
        let persisted = db.workspace_environment_rows("node-one").unwrap().remove(0);
        assert_eq!(persisted.status, "building");
        assert_eq!(persisted.reason.as_deref(), Some("sentinel"));
        assert_eq!(persisted.updated_at, -1);

        let paths = db.workspace_view_paths_for_lane("node-one").unwrap();
        fs::write(
            paths.source_upper.join("package-lock.json"),
            r#"{"name":"trail-node-layer-test","version":"1.0.1","lockfileVersion":3,"requires":true,"packages":{"":{"name":"trail-node-layer-test","version":"1.0.1"}}}"#,
        )
        .unwrap();
        db.checkpoint_lane_workspace("node-one", Some("lock changed".to_string()))
            .unwrap();
        let readiness = db.lane_readiness("node-one").unwrap();
        assert!(readiness
            .blockers
            .iter()
            .any(|issue| issue.code == "dependency_environment_stale"));
    }
}
