use super::*;

impl Trail {
    pub(crate) fn ensure_workspace_git_shadow(
        &self,
        view: &LaneWorkspaceViewReport,
        source_root: &ObjectId,
    ) -> Result<Option<WorkspaceGitShadowReport>> {
        if let Some(shadow) = self.workspace_git_shadow(view)? {
            return self.refresh_workspace_git_shadow(&shadow).map(Some);
        }
        let pinned_head = self
            .conn
            .query_row(
                "SELECT git_head FROM git_mappings WHERE crab_root = ?1 AND git_dirty = 0 AND git_head IS NOT NULL ORDER BY created_at DESC LIMIT 1",
                params![source_root.0],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(pinned_head) = pinned_head else {
            return Ok(None);
        };
        let common_dir = git_real_common_dir(&self.workspace_root)?;
        let object_dir = common_dir.join("objects").canonicalize()?;
        let git_dir = self.db_dir.join("git-shadows").join(&view.view_id);
        if git_dir.exists() && fs::read_dir(&git_dir)?.next().transpose()?.is_some() {
            return Err(Error::InvalidInput(format!(
                "Git shadow directory `{}` contains untracked recovery state",
                git_dir.display()
            )));
        }
        fs::create_dir_all(&git_dir)?;
        let init = Command::new("git")
            .args(["init", "--bare", "--quiet"])
            .arg(&git_dir)
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !init.status.success() {
            return Err(Error::Git(format!(
                "failed to initialize Git shadow: {}",
                String::from_utf8_lossy(&init.stderr).trim()
            )));
        }
        fs::create_dir_all(git_dir.join("objects/info"))?;
        write_file_atomic(
            &git_dir.join("objects/info/alternates"),
            format!("{}\n", object_dir.display()).as_bytes(),
            false,
        )?;
        fs::create_dir_all(git_dir.join("refs/heads"))?;
        write_file_atomic(
            &git_dir.join("refs/heads/trail-view"),
            format!("{pinned_head}\n").as_bytes(),
            false,
        )?;
        write_file_atomic(
            &git_dir.join("HEAD"),
            b"ref: refs/heads/trail-view\n",
            false,
        )?;
        git_shadow_command(
            &git_dir,
            Path::new(&view.mountpoint),
            &["config", "core.bare", "false"],
        )?;
        git_shadow_command(
            &git_dir,
            Path::new(&view.mountpoint),
            &["config", "core.worktree", &view.mountpoint],
        )?;
        git_shadow_command(
            &git_dir,
            Path::new(&view.mountpoint),
            &["config", "advice.detachedHead", "false"],
        )?;
        git_shadow_command(
            &git_dir,
            Path::new(&view.mountpoint),
            &["read-tree", &pinned_head],
        )?;
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO workspace_git_shadows (view_id, git_dir, policy, pinned_head, current_head, status, created_at, updated_at) VALUES (?1, ?2, 'status', ?3, ?3, 'ready', ?4, ?4)",
            params![view.view_id, git_dir.to_string_lossy(), pinned_head, now],
        )?;
        self.workspace_git_shadow(view)
    }

    pub(crate) fn workspace_git_shadow(
        &self,
        view: &LaneWorkspaceViewReport,
    ) -> Result<Option<WorkspaceGitShadowReport>> {
        self.conn
            .query_row(
                "SELECT view_id, git_dir, policy, pinned_head, current_head, status, updated_at FROM workspace_git_shadows WHERE view_id = ?1",
                params![view.view_id],
                |row| {
                    Ok(WorkspaceGitShadowReport {
                        view_id: row.get(0)?,
                        git_dir: row.get(1)?,
                        work_tree: view.mountpoint.clone(),
                        policy: row.get(2)?,
                        pinned_head: row.get(3)?,
                        current_head: row.get(4)?,
                        status: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn refresh_workspace_git_shadow(
        &self,
        shadow: &WorkspaceGitShadowReport,
    ) -> Result<WorkspaceGitShadowReport> {
        let current_head = git_shadow_command(
            Path::new(&shadow.git_dir),
            Path::new(&shadow.work_tree),
            &["rev-parse", "--verify", "HEAD"],
        )?;
        let status = if current_head == shadow.pinned_head {
            "ready"
        } else {
            "diverged"
        };
        self.conn.execute(
            "UPDATE workspace_git_shadows SET current_head = ?1, status = ?2, updated_at = ?3 WHERE view_id = ?4",
            params![current_head, status, now_ts(), shadow.view_id],
        )?;
        Ok(WorkspaceGitShadowReport {
            current_head,
            status: status.to_string(),
            updated_at: now_ts(),
            ..shadow.clone()
        })
    }
}

fn git_real_common_dir(workspace: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .map_err(|err| Error::Git(err.to_string()))?;
    if !output.status.success() {
        return Err(Error::Git(format!(
            "failed to locate real Git object directory: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

fn git_shadow_command(git_dir: &Path, work_tree: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_DIR", git_dir)
        .env("GIT_WORK_TREE", work_tree)
        .env("GIT_INDEX_FILE", git_dir.join("index"))
        .env("GIT_AUTHOR_NAME", "Trail Workspace View")
        .env("GIT_AUTHOR_EMAIL", "trail@local.invalid")
        .env("GIT_COMMITTER_NAME", "Trail Workspace View")
        .env("GIT_COMMITTER_EMAIL", "trail@local.invalid")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .map_err(|err| Error::Git(err.to_string()))?;
    if !output.status.success() {
        return Err(Error::Git(format!(
            "shadow git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_git_ref_changes_never_touch_real_git_and_block_readiness() {
        let workspace = tempfile::tempdir().unwrap();
        Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(workspace.path())
            .status()
            .unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        for args in [
            vec!["config", "user.name", "Trail Test"],
            vec!["config", "user.email", "trail@example.invalid"],
            vec!["add", "README.md"],
            vec!["commit", "--quiet", "-m", "base"],
        ] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(workspace.path())
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        Trail::init(workspace.path(), "main", InitImportMode::GitTracked, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::OverlayCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "git-shadow",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let view = db.lane_workspace_view("git-shadow").unwrap().unwrap();
        let branch = db.lane_branch("git-shadow").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let shadow = db
            .ensure_workspace_git_shadow(&view, &head.root_id)
            .unwrap()
            .unwrap();
        let real_head = Command::new("git")
            .arg("-C")
            .arg(workspace.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let empty_tree = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
        let new_commit = git_shadow_command(
            Path::new(&shadow.git_dir),
            Path::new(&shadow.work_tree),
            &["commit-tree", empty_tree, "-m", "shadow"],
        )
        .unwrap();
        git_shadow_command(
            Path::new(&shadow.git_dir),
            Path::new(&shadow.work_tree),
            &["update-ref", "HEAD", &new_commit],
        )
        .unwrap();
        let real_after = Command::new("git")
            .arg("-C")
            .arg(workspace.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(real_head.stdout, real_after.stdout);
        assert_eq!(
            db.refresh_workspace_git_shadow(&shadow).unwrap().status,
            "diverged"
        );
        let readiness = db.lane_readiness("git-shadow").unwrap();
        assert!(readiness
            .blockers
            .iter()
            .any(|issue| issue.code == "shadow_git_head_diverged"));
    }
}
