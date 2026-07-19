use super::*;

impl Trail {
    pub fn checkout(&mut self, change_or_ref: &str, force: bool) -> Result<CheckoutReport> {
        self.checkout_with_options(change_or_ref, force, false, None, false)
    }

    pub fn checkout_with_options(
        &mut self,
        change_or_ref: &str,
        force: bool,
        dry_run: bool,
        workdir: Option<&Path>,
        record_dirty: bool,
    ) -> Result<CheckoutReport> {
        // TRAIL_FS_PRODUCER: primary_workspace_checkout Checkout controlled
        let ledger_authority =
            crate::db::change_ledger::command_authority_enabled() && workdir.is_none() && !dry_run;
        let _lock = if ledger_authority {
            None
        } else {
            Some(self.acquire_write_lock()?)
        };
        if dry_run && record_dirty {
            return Err(Error::InvalidInput(
                "checkout --record-dirty cannot be combined with --dry-run".to_string(),
            ));
        }
        let mut recorded_dirty = None;
        if record_dirty {
            let current_branch = self.current_branch()?;
            let report = self.record_with_options_unlocked(
                Some(&current_branch),
                Some(format!(
                    "Record dirty worktree before checkout `{change_or_ref}`"
                )),
                Actor::human(),
                RecordOptions {
                    kind: Some(OperationKind::Checkout),
                    ..RecordOptions::default()
                },
            )?;
            recorded_dirty = report.operation;
        }
        let current = self.resolve_branch_ref(&self.current_branch()?)?;
        if !dry_run && workdir.is_none() && !force && !record_dirty {
            let status = self.status(None)?;
            if status.worktree_state != WorktreeState::Clean {
                return Err(Error::DirtyWorktree);
            }
        }
        let target = self.resolve_refish(change_or_ref)?;
        let output_root = workdir
            .map(|path| self.resolve_checkout_workdir_path(path))
            .transpose()?;
        if target.root_id == current.root_id
            && let Some(output_root) = &output_root
        {
            // TRAIL_FS_PRODUCER: alternate_checkout_destination exempt_alternate_output exempt
            let written_files = if dry_run {
                0
            } else {
                prepare_checkout_workdir(output_root)?;
                self.materialize_root_files_at_streaming(&target.root_id, output_root, true)?
                    .file_count
            };
            return Ok(CheckoutReport {
                change_id: target.change_id,
                root_id: target.root_id,
                written_files,
                dry_run,
                recorded_dirty,
                output_root: Some(output_root.to_string_lossy().to_string()),
                changed_paths: Vec::new(),
            });
        }
        let current_files = self.load_root_files(&current.root_id)?;
        let target_files = self.load_root_files(&target.root_id)?;
        let diff = self.diff_file_maps(&current_files, &target_files)?;
        if !dry_run {
            if let Some(output_root) = &output_root {
                prepare_checkout_workdir(output_root)?;
                let cloned_from_workspace = if target.root_id == current.root_id {
                    let source_stamps = match self.workspace_file_stamps_if_clean_index_matches(
                        &target.root_id,
                        &target_files,
                    )? {
                        Some(stamps) => Some(stamps),
                        None => self.workspace_file_stamps_if_entries_match(&target_files)?,
                    };
                    if let Some(source_stamps) = source_stamps {
                        materialize_from_workspace_cow(
                            &self.workspace_root,
                            output_root,
                            &target_files,
                            &source_stamps,
                            true,
                        )?
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !cloned_from_workspace {
                    self.materialize_files_at(output_root, &BTreeMap::new(), &target_files)?;
                }
            } else if ledger_authority {
                let expected =
                    crate::db::change_ledger::prepare_workspace_controlled_projection(self)?;
                let mut evidence_paths = diff
                    .summaries
                    .iter()
                    .flat_map(|summary| {
                        std::iter::once(summary.path.as_str()).chain(summary.old_path.as_deref())
                    })
                    .map(crate::db::change_ledger::LedgerPath::parse)
                    .collect::<Result<Vec<_>>>()?;
                evidence_paths.sort();
                evidence_paths.dedup();
                let evidence = crate::db::change_ledger::IntentEvidence {
                    exact_paths: evidence_paths,
                    complete_prefixes: Vec::new(),
                };
                let alignment = if target.root_id == current.root_id {
                    crate::db::change_ledger::ProjectionAlignmentMode::Aligned
                } else {
                    crate::db::change_ledger::ProjectionAlignmentMode::RetainDirty {
                        target: crate::db::change_ledger::IntentTarget {
                            change_id: target.change_id.clone(),
                            root_id: target.root_id.clone(),
                            operation_id: None,
                        },
                    }
                };
                crate::db::change_ledger::run_projection_alignment(
                    self,
                    &expected,
                    crate::db::change_ledger::IntentProducer::Checkout,
                    &evidence,
                    alignment,
                    |db, intent| {
                        crate::db::change_ledger::with_workspace_controlled_interval(
                            db,
                            intent,
                            &evidence,
                            |db| {
                                if force {
                                    db.remove_visible_files_absent_from_target(&target_files)?;
                                }
                                db.materialize_files(&current_files, &target_files)
                            },
                            |db, policy, candidates| {
                                let comparison = db.compare_controlled_projection_target(
                                    policy,
                                    candidates,
                                    &target.root_id,
                                    crate::db::change_ledger::CandidateMaterialization::ManifestOnly,
                                )?;
                                if comparison.summaries.is_empty() {
                                    Ok(())
                                } else {
                                    Err(Error::ChangeLedgerReconcileRequired {
                                        scope: expected.scope_id.to_text(),
                                        state: "stale_baseline".into(),
                                        reason: "checkout pinned verification did not match its target root".into(),
                                        command: "trail status".into(),
                                    })
                                }
                            },
                        )
                    },
                    |_| Ok(()),
                )?;
            } else {
                if force {
                    self.remove_visible_files_absent_from_target(&target_files)?;
                }
                self.materialize_files(&current_files, &target_files)?;
            }
        }
        Ok(CheckoutReport {
            change_id: target.change_id,
            root_id: target.root_id,
            written_files: if dry_run {
                0
            } else {
                target_files.len() as u64
            },
            dry_run,
            recorded_dirty,
            output_root: output_root.map(|path| path.to_string_lossy().to_string()),
            changed_paths: diff.summaries,
        })
    }

    fn remove_visible_files_absent_from_target(
        &self,
        target_files: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        let mut changed_directories = BTreeSet::new();
        for path in self.scan_worktree_file_paths()?.paths {
            if target_files.contains_key(&path) {
                continue;
            }
            let abs = safe_join(&self.workspace_root, &path)?;
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            fs::remove_file(&abs)?;
            let parent = abs.parent().ok_or_else(|| Error::InvalidPath {
                path: path.clone(),
                reason: "removed workspace file has no parent directory".into(),
            })?;
            changed_directories.insert(parent.to_path_buf());
        }
        // The controlled projection proof is not allowed to advance until
        // forced deletions are durable. Sync each directory whose namespace
        // changed so a successful return means the removals survive a crash.
        for directory in changed_directories {
            sync_directory_strict(&directory)?;
        }
        Ok(())
    }
}
