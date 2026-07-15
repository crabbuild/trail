#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::change_ledger::{
        BaselineIdentity, ChangedPathLedger, ExpectedScope, FilesystemIdentity, PolicyIdentity,
        ProviderCapabilities, ProviderIdentity, ScopeId, ScopeIdentity, ScopeKind,
    };
    use crate::ids::{ChangeId, ObjectId};
    use crate::{InitImportMode, Trail};
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    struct Fixture {
        _temp: tempfile::TempDir,
        db: Trail,
        expected: ExpectedScope,
        git_env: Vec<(OsString, OsString)>,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            run_git(root, &["init", "--quiet"]);
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(root.join("src/.gitignore"), "generated\n").unwrap();
            fs::write(root.join("src/.trailignore"), "private\n").unwrap();
            Trail::init(root, "main", InitImportMode::Empty, false).unwrap();
            let db = Trail::open(root).unwrap();
            let scope = ScopeIdentity {
                scope_id: ScopeId([7; 32]),
                kind: ScopeKind::Workspace,
                owner_id: "policy-test".to_string(),
            };
            let baseline = BaselineIdentity {
                ref_name: "main".to_string(),
                ref_generation: 1,
                change_id: ChangeId("change".to_string()),
                root_id: ObjectId("root".to_string()),
            };
            let policy = PolicyIdentity {
                fingerprint: [0; 32],
                generation: 1,
            };
            let filesystem = FilesystemIdentity(vec![1]);
            let provider = ProviderIdentity {
                identity: vec![2],
                capabilities: ProviderCapabilities {
                    durable_cursor: false,
                    linearizable_fence: false,
                    rename_pairing: false,
                    overflow_scope: false,
                    filesystem_supported: false,
                    clean_proof_allowed: false,
                    power_loss_durability: false,
                },
            };
            ChangedPathLedger::new(&db.conn)
                .begin_scope(&scope, &baseline, &policy, &filesystem, &provider)
                .unwrap();
            let expected = ExpectedScope {
                scope_id: scope.scope_id,
                epoch: 1,
                ref_name: baseline.ref_name,
                ref_generation: baseline.ref_generation,
                baseline_root: baseline.root_id,
                policy_fingerprint: policy.fingerprint,
                policy_generation: policy.generation,
                filesystem_identity: filesystem.0,
                provider_identity: provider.identity,
            };
            Self {
                _temp: temp,
                db,
                expected,
                git_env: vec![("GIT_CONFIG_NOSYSTEM".into(), "1".into())],
            }
        }

        fn root(&self) -> &Path {
            &self.db.workspace_root
        }

        fn context(&self) -> PolicyCompileContext<'_> {
            PolicyCompileContext {
                workspace_root: &self.db.workspace_root,
                db_dir: &self.db.db_dir,
                recording: &self.db.config.recording,
                case_sensitive: true,
                git_environment: &self.git_env,
            }
        }

        fn compile(&self, metrics: &mut PolicyDependencyMetrics) -> CompiledPolicy {
            compile_policy(&self.db.conn, &self.expected, &self.context(), metrics).unwrap()
        }
    }

    #[test]
    fn raw_nested_ignore_event_stales_scope_before_ignore_filtering() {
        let fixture = Fixture::new();
        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());

        assert!(raw_event_invalidates_policy(
            &policy,
            &fixture.root().join("src/.gitignore")
        ));
        assert!(raw_event_invalidates_policy(
            &policy,
            &fixture.root().join("src/.trailignore")
        ));
        assert!(raw_path_may_invalidate_policy(Path::new(
            ".trail/config.toml"
        )));
        let nested_rules = policy
            .snapshot
            .rule_sources
            .iter()
            .find(|source| source.path == fixture.root().join("src/.gitignore"))
            .expect("compiled snapshot pins semantic rule bytes");
        assert_eq!(nested_rules.bytes, b"generated\n");
        assert!(policy
            .dependencies
            .iter()
            .filter(|dependency| dependency_is_file(dependency))
            .all(|dependency| !dependency.observable && dependency.last_source_sequence == 0));
    }

    #[test]
    fn unchanged_manifest_reuses_policy_without_tree_discovery() {
        let fixture = Fixture::new();
        let mut metrics = PolicyDependencyMetrics::default();

        let first = fixture.compile(&mut metrics);
        let second = fixture.compile(&mut metrics);

        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(metrics.policy_dependency_full_discovery, 1);
        assert!(second.reused_manifest);
        assert!(second.stale_baseline);
        assert_eq!(second.adapter_equivalence, AdapterEquivalence::Conservative);
        let persisted: i64 = fixture
            .db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM changed_path_policy_dependencies WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted as usize, second.dependencies.len());
    }

    #[test]
    fn no_observer_proof_never_returns_equivalent_even_when_direct_checks_reuse_manifest() {
        let fixture = Fixture::new();
        let mut metrics = PolicyDependencyMetrics::default();
        let _first = fixture.compile(&mut metrics);
        fs::create_dir_all(fixture.root().join("new/nested")).unwrap();
        fs::write(fixture.root().join("new/nested/.gitignore"), "late\n").unwrap();

        let reused = fixture.compile(&mut metrics);

        assert!(
            reused.reused_manifest,
            "bounded direct reuse remains permitted"
        );
        assert!(reused.stale_baseline);
        assert_eq!(reused.adapter_equivalence, AdapterEquivalence::Conservative);
        assert_eq!(metrics.policy_dependency_full_discovery, 1);
    }

    #[test]
    fn fabricated_observer_cut_cannot_promote_synthetic_only_manifest() {
        let fixture = Fixture::new();
        let context = fixture.context();
        let observer_cut = QualifiedPolicyObserverCut {
            scope_id: fixture.expected.scope_id,
            provider_identity: fixture.expected.provider_identity.clone(),
            discovery_started_sequence: 1,
            through_sequence: 1,
            covered_roots: vec![fixture.root().to_path_buf()],
            case_sensitive: true,
        };
        assert!(observer_cut.validate_for(
            &fixture.expected,
            context.workspace_root,
            context.case_sensitive,
        ));
        let manifest = PolicyManifest {
            dependencies: synthetic_dependencies(fixture.expected.policy_generation, &context)
                .unwrap(),
            generation: fixture.expected.policy_generation,
            rule_sources: Vec::new(),
        };
        let policy = finish_compiled_policy(&context, manifest.clone(), false).unwrap();

        assert_eq!(policy.adapter_equivalence, AdapterEquivalence::Conservative);
        assert!(policy.stale_baseline);

        persist_policy_manifest_rows(&fixture.db.conn, &fixture.expected, &manifest).unwrap();
        let compiled = fixture.compile(&mut PolicyDependencyMetrics::default());
        assert!(compiled.reused_manifest);
        assert_eq!(
            compiled.adapter_equivalence,
            AdapterEquivalence::Conservative
        );
        assert!(compiled.stale_baseline);
        let (trust_state, continuity_generation): (String, i64) = fixture
            .db
            .conn
            .query_row(
                "SELECT trust_state,continuity_generation
                 FROM changed_path_scopes WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(trust_state, "stale_baseline");
        assert!(continuity_generation > 1);
    }

    #[test]
    fn production_compiled_policy_cannot_authorize_reconciliation() {
        let fixture = Fixture::new();
        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());

        assert!(!policy.authorizes_reconciliation(&fixture.expected));
        assert_eq!(policy.adapter_equivalence, AdapterEquivalence::Conservative);
        assert!(policy.stale_baseline);
    }

    #[test]
    fn exact_invalidation_index_matches_arbitrary_dependency_with_case_policy() {
        let fixture = Fixture::new();
        let mut policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        let arbitrary = fixture.root().join("config/Arbitrary.Rules");
        fs::create_dir_all(arbitrary.parent().unwrap()).unwrap();
        fs::write(&arbitrary, "*.generated\n").unwrap();
        policy.dependencies.push(
            file_dependency(
                &arbitrary,
                PolicyDependencyKind::GitExcludesFile,
                policy.manifest().generation,
                &fixture.context(),
            )
            .unwrap(),
        );
        policy.invalidation_index =
            PolicyInvalidationIndex::from_dependencies(fixture.root(), false, &policy.dependencies)
                .unwrap();

        assert!(raw_event_invalidates_policy(
            &policy,
            Path::new("CONFIG/arbitrary.rules")
        ));
        assert!(!raw_event_invalidates_policy(
            &policy,
            Path::new("config/unrelated.rules")
        ));
    }

    #[test]
    fn default_missing_git_candidates_and_missing_include_target_are_authoritative() {
        let mut fixture = Fixture::new();
        let home = fixture.root().parent().unwrap().join("home");
        let xdg = fixture.root().parent().unwrap().join("xdg");
        fs::create_dir_all(home.join(".config/git")).unwrap();
        fs::create_dir_all(xdg.join("git")).unwrap();
        let global = home.join(".gitconfig");
        let missing_include = home.join("not-created-yet.gitconfig");
        fs::write(
            &global,
            format!("[include]\n\tpath = {}\n", missing_include.display()),
        )
        .unwrap();
        fixture.git_env = vec![
            ("HOME".into(), home.as_os_str().to_owned()),
            ("XDG_CONFIG_HOME".into(), xdg.as_os_str().to_owned()),
            ("GIT_CONFIG_NOSYSTEM".into(), "0".into()),
            (
                "GIT_CONFIG_SYSTEM".into(),
                fixture
                    .root()
                    .parent()
                    .unwrap()
                    .join("system.gitconfig")
                    .into_os_string(),
            ),
        ];

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        for candidate in [
            home.join(".gitconfig"),
            xdg.join("git/config"),
            fixture.root().parent().unwrap().join("system.gitconfig"),
            xdg.join("git/ignore"),
            missing_include.clone(),
        ] {
            assert!(
                policy
                    .dependencies
                    .iter()
                    .any(|dependency| dependency.identity
                        == dependency_path_identity_with_case(&candidate, true)),
                "missing candidate {}",
                candidate.display()
            );
        }
        fs::write(&missing_include, "[core]\n\texcludesFile = ignored\n").unwrap();
        assert_eq!(
            validate_policy_manifest(&fixture.context(), &policy.manifest()).unwrap(),
            PolicyManifestValidation::Changed
        );
    }

    #[test]
    fn config_selector_environment_change_invalidates_direct_reuse() {
        let mut fixture = Fixture::new();
        let first = fixture.compile(&mut PolicyDependencyMetrics::default());
        fixture.git_env.push((
            "XDG_CONFIG_HOME".into(),
            fixture.root().join("different-xdg").into_os_string(),
        ));

        assert_eq!(
            validate_policy_manifest(&fixture.context(), &first.manifest()).unwrap(),
            PolicyManifestValidation::Changed
        );
    }

    #[test]
    fn command_scope_missing_includes_are_authoritative_dependencies() {
        let mut fixture = Fixture::new();
        let counted = fixture.root().join("counted/missing.gitconfig");
        let parameter = fixture.root().join("parameter/missing.gitconfig");
        fixture.git_env.extend([
            ("GIT_CONFIG_COUNT".into(), "2".into()),
            ("GIT_CONFIG_KEY_0".into(), "include.path".into()),
            ("GIT_CONFIG_VALUE_0".into(), counted.as_os_str().to_owned()),
            (
                "GIT_CONFIG_KEY_1".into(),
                "includeIf.gitdir:/**.path".into(),
            ),
            (
                "GIT_CONFIG_VALUE_1".into(),
                fixture
                    .root()
                    .join("counted/conditional.gitconfig")
                    .into_os_string(),
            ),
            (
                "GIT_CONFIG_PARAMETERS".into(),
                OsString::from(format!("'include.path'='{}'", parameter.display())),
            ),
        ]);

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        for target in [
            counted,
            fixture.root().join("counted/conditional.gitconfig"),
            parameter,
        ] {
            let dependency = policy
                .dependencies
                .iter()
                .find(|dependency| {
                    dependency.identity == dependency_path_identity_with_case(&target, true)
                })
                .unwrap_or_else(|| panic!("missing injected include {}", target.display()));
            assert!(!dependency.observable);
            assert!(policy.invalidation_index.matches(fixture.root(), &target));
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, "[core]\n\tignoreCase = false\n").unwrap();
            assert_eq!(
                validate_policy_manifest(&fixture.context(), &policy.manifest()).unwrap(),
                PolicyManifestValidation::Changed
            );
            fs::remove_file(&target).unwrap();
        }
    }

    #[test]
    fn empty_and_ascii_padded_git_config_count_keep_indexed_includes() {
        let mut fixture = Fixture::new();
        let target = fixture.root().join("indexed/missing.gitconfig");
        fixture.git_env.extend([
            ("GIT_CONFIG_COUNT".into(), "  1\t".into()),
            ("GIT_CONFIG_KEY_0".into(), "include.path".into()),
            ("GIT_CONFIG_VALUE_0".into(), target.as_os_str().to_owned()),
        ]);

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        assert!(policy.dependencies.iter().any(|dependency| {
            dependency.identity == dependency_path_identity_with_case(&target, true)
        }));

        fixture.git_env.retain(|(key, _)| {
            !matches!(
                key.to_str(),
                Some("GIT_CONFIG_KEY_0" | "GIT_CONFIG_VALUE_0")
            )
        });
        fixture.git_env.iter_mut().for_each(|(key, value)| {
            if key == OsStr::new("GIT_CONFIG_COUNT") {
                *value = OsString::new();
            }
        });
        assert!(discover_git_dependencies(1, &fixture.context()).is_ok());
    }

    #[test]
    fn git_selector_paths_follow_git_cwd_and_empty_semantics() {
        let mut fixture = Fixture::new();
        let home = fixture.root().join("home");
        fixture.git_env = vec![
            ("HOME".into(), home.as_os_str().to_owned()),
            ("XDG_CONFIG_HOME".into(), OsString::new()),
            ("GIT_CONFIG_GLOBAL".into(), OsString::new()),
            ("GIT_CONFIG_SYSTEM".into(), "config/system.gitconfig".into()),
        ];

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        let identities = policy
            .dependencies
            .iter()
            .map(|dependency| dependency.identity.as_str())
            .collect::<BTreeSet<_>>();

        assert!(identities.contains(
            dependency_path_identity_with_case(&home.join(".config/git/ignore"), true).as_str()
        ));
        assert!(identities.contains(
            dependency_path_identity_with_case(
                &fixture.root().join("config/system.gitconfig"),
                true,
            )
            .as_str()
        ));
        assert!(!identities
            .contains(dependency_path_identity_with_case(&home.join(".gitconfig"), true).as_str()));
        assert!(!identities.contains(
            dependency_path_identity_with_case(&home.join(".config/git/config"), true).as_str()
        ));
    }

    #[test]
    fn empty_home_uses_root_based_global_and_xdg_candidates() {
        let mut fixture = Fixture::new();
        fixture.git_env.extend([
            ("HOME".into(), OsString::new()),
            ("XDG_CONFIG_HOME".into(), OsString::new()),
        ]);

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        for target in [
            PathBuf::from("/.gitconfig"),
            PathBuf::from("/.config/git/config"),
            PathBuf::from("/.config/git/ignore"),
        ] {
            assert!(policy.dependencies.iter().any(|dependency| {
                dependency.identity == dependency_path_identity_with_case(&target, true)
            }));
        }
    }

    #[test]
    fn relative_xdg_and_global_paths_resolve_from_git_cwd() {
        let mut fixture = Fixture::new();
        fixture.git_env = vec![
            ("XDG_CONFIG_HOME".into(), "xdg-relative".into()),
            ("GIT_CONFIG_GLOBAL".into(), "config/global.gitconfig".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        for target in [
            fixture.root().join("xdg-relative/git/ignore"),
            fixture.root().join("config/global.gitconfig"),
        ] {
            assert!(policy.dependencies.iter().any(|dependency| {
                dependency.identity == dependency_path_identity_with_case(&target, true)
            }));
        }
    }

    #[test]
    fn git_subprocess_environment_drops_every_ambient_repository_selector() {
        let mut fixture = Fixture::new();
        fixture.git_env.extend([
            ("HOME".into(), fixture.root().join("home").into_os_string()),
            (
                "GIT_DIR".into(),
                fixture.root().join(".git").into_os_string(),
            ),
        ]);
        let ambient = vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("GIT_CONFIG".into(), "/tmp/hostile-config".into()),
            ("GIT_COMMON_DIR".into(), "/tmp/hostile-common".into()),
            ("GIT_WORK_TREE".into(), "/tmp/hostile-worktree".into()),
            ("GIT_OBJECT_DIRECTORY".into(), "/tmp/hostile-objects".into()),
            ("HOME".into(), "/tmp/hostile-home".into()),
            ("XDG_CONFIG_HOME".into(), "/tmp/hostile-xdg".into()),
            ("UNRELATED_AMBIENT".into(), "not-needed".into()),
        ];

        let environment = git_command_environment(&fixture.context(), ambient);
        let keys = environment
            .iter()
            .map(|(key, _)| key.as_os_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(
            keys,
            BTreeSet::from([
                OsStr::new("PATH"),
                OsStr::new("HOME"),
                OsStr::new("GIT_DIR"),
                OsStr::new("GIT_CONFIG_NOSYSTEM"),
            ])
        );
        let expected_home = fixture.root().join("home").into_os_string();
        assert_eq!(
            environment
                .iter()
                .find(|(key, _)| key == OsStr::new("HOME"))
                .map(|(_, value)| value),
            Some(&expected_home)
        );
    }

    #[test]
    fn canonical_fingerprint_is_framed_and_rejects_duplicate_identities() {
        let fixture = Fixture::new();
        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        let mut duplicate = policy.manifest();
        duplicate
            .dependencies
            .push(duplicate.dependencies[0].clone());
        assert!(validate_policy_manifest(&fixture.context(), &duplicate).is_err());

        let mut left = policy.dependencies[0].clone();
        let mut right = policy.dependencies[0].clone();
        left.identity = "ab".into();
        right.identity = "a".into();
        let left_fingerprint = policy_fingerprint(&[left.clone(), {
            let mut x = right.clone();
            x.identity = "c".into();
            x
        }])
        .unwrap();
        let right_fingerprint = policy_fingerprint(&[
            {
                left.identity = "a".into();
                left
            },
            {
                right.identity = "bc".into();
                right
            },
        ])
        .unwrap();
        assert_ne!(left_fingerprint, right_fingerprint);
    }

    #[test]
    fn stale_expected_scope_cannot_delete_concurrent_manifest_replacement() {
        let fixture = Fixture::new();
        let first = fixture.compile(&mut PolicyDependencyMetrics::default());
        fixture
            .db
            .conn
            .execute(
                "UPDATE changed_path_scopes SET epoch=epoch+1 WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
            )
            .unwrap();
        let mut replacement_expected = fixture.expected.clone();
        replacement_expected.epoch += 1;
        let replacement =
            discover_policy_manifest(replacement_expected.policy_generation, &fixture.context())
                .unwrap();
        persist_policy_manifest_and_stale(&fixture.db.conn, &replacement_expected, &replacement)
            .unwrap();

        let stale_result = persist_policy_manifest_and_stale(
            &fixture.db.conn,
            &fixture.expected,
            &first.manifest(),
        );
        assert!(stale_result.is_err());
        let count: i64 = fixture
            .db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM changed_path_policy_dependencies WHERE scope_id=?1",
                [replacement_expected.scope_id.to_text()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count as usize, replacement.dependencies.len());
    }

    #[test]
    fn direct_nested_dependency_change_is_detected_and_marks_scope_stale() {
        let fixture = Fixture::new();
        let mut metrics = PolicyDependencyMetrics::default();
        let first = fixture.compile(&mut metrics);
        fs::write(fixture.root().join("src/.gitignore"), "different\n").unwrap();

        assert_eq!(
            validate_policy_manifest(&fixture.context(), &first.manifest()).unwrap(),
            PolicyManifestValidation::Changed
        );
        let second = fixture.compile(&mut metrics);

        assert_ne!(first.fingerprint, second.fingerprint);
        assert!(second.stale_baseline);
        assert_eq!(metrics.policy_dependency_full_discovery, 2);
        let state: String = fixture
            .db
            .conn
            .query_row(
                "SELECT trust_state FROM changed_path_scopes WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "stale_baseline");
    }

    #[test]
    fn git_config_includes_core_excludes_and_global_origins_are_dependencies() {
        let mut fixture = Fixture::new();
        let global = fixture.root().parent().unwrap().join("global.gitconfig");
        let included = fixture.root().parent().unwrap().join("included.gitconfig");
        let excludes = fixture.root().parent().unwrap().join("global-excludes");
        fs::write(&excludes, "*.cache\n").unwrap();
        fs::write(
            &included,
            format!("[core]\n\texcludesFile = {}\n", excludes.display()),
        )
        .unwrap();
        fs::write(
            &global,
            format!("[include]\n\tpath = {}\n", included.display()),
        )
        .unwrap();
        fixture
            .git_env
            .push(("GIT_CONFIG_GLOBAL".into(), global.as_os_str().to_owned()));

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());

        for path in [&global, &included, &excludes] {
            let identity = dependency_path_identity(path);
            let dependency = policy
                .dependencies
                .iter()
                .find(|dependency| dependency.identity == identity)
                .unwrap_or_else(|| panic!("missing dependency {}", path.display()));
            assert!(!dependency.observable);
        }
        assert!(policy.stale_baseline);
        assert_eq!(policy.adapter_equivalence, AdapterEquivalence::Conservative);
    }

    #[test]
    fn git_decodes_include_values_and_resolves_them_from_the_origin_file() {
        let mut fixture = Fixture::new();
        let config_root = fixture.root().parent().unwrap().join("git-config-grammar");
        fs::create_dir_all(&config_root).unwrap();
        let global = config_root.join("global.gitconfig");
        fs::write(
            &global,
            concat!(
                "[include]\n",
                "\tpath = \"quoted dir/missing include.conf\" # trailing comment\n",
                "[include]\n",
                "\tpath = continued-\\\n",
                "target.conf\n",
                "[includeIf \"gitdir:/**\"]\n",
                "\tpath = \"tab\\tmissing.conf\"\n",
            ),
        )
        .unwrap();
        fixture
            .git_env
            .push(("GIT_CONFIG_GLOBAL".into(), global.into_os_string()));

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());

        for target in [
            config_root.join("quoted dir/missing include.conf"),
            config_root.join("continued-target.conf"),
            config_root.join("tab\tmissing.conf"),
        ] {
            assert!(
                policy.dependencies.iter().any(|dependency| {
                    dependency.identity == dependency_path_identity_with_case(&target, true)
                }),
                "Git-decoded missing include was not persisted: {}",
                target.display()
            );
        }
    }

    #[test]
    fn git_expands_policy_paths_before_origin_or_cwd_resolution() {
        let mut fixture = Fixture::new();
        let home = fixture.root().parent().unwrap().join("path-expansion-home");
        let config_root = fixture
            .root()
            .parent()
            .unwrap()
            .join("path-expansion-config");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&config_root).unwrap();
        let global = config_root.join("global.gitconfig");
        fs::write(
            &global,
            concat!(
                "[include]\n",
                "\tpath = \"%(prefix)/share/trail approval/include.conf\"\n",
                "[includeIf \"gitdir:/**\"]\n",
                "\tpath = \"~/quoted dir/conditional.conf\"\n",
                "[core]\n",
                "\texcludesFile = \"~/excluded-\\\n",
                "rules\"\n",
            ),
        )
        .unwrap();
        fixture.git_env.extend([
            ("HOME".into(), home.into_os_string()),
            ("GIT_CONFIG_GLOBAL".into(), global.as_os_str().to_owned()),
        ]);

        let git_path = |key: &str| {
            let output = Command::new("git")
                .args(["config", "--file"])
                .arg(&global)
                .args(["--path", "--get-all", key])
                .current_dir(fixture.root())
                .env_clear()
                .envs(git_command_environment(
                    &fixture.context(),
                    std::env::vars_os(),
                ))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            PathBuf::from(os_string_from_bytes(
                output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout),
            ))
        };
        let expected = [
            git_path("include.path"),
            git_path("includeIf.gitdir:/**.path"),
            git_path("core.excludesFile"),
        ];

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());

        for target in expected {
            let target = if target.is_absolute() {
                target
            } else {
                config_root.join(target)
            };
            assert!(
                policy.dependencies.iter().any(|dependency| {
                    dependency.identity == dependency_path_identity_with_case(&target, true)
                }),
                "missing Git-expanded path {}",
                target.display()
            );
        }
    }

    #[test]
    fn legacy_git_config_file_and_its_policy_keys_are_dependencies() {
        let mut fixture = Fixture::new();
        let config_dir = fixture.root().join("config");
        fs::create_dir_all(&config_dir).unwrap();
        let legacy = config_dir.join("legacy.gitconfig");
        let included = config_dir.join("included.gitconfig");
        fs::write(
            &legacy,
            "[include]\n\tpath = included.gitconfig\n[core]\n\texcludesFile = legacy.ignore\n",
        )
        .unwrap();
        fs::write(&included, "[core]\n\texcludesFile = included.ignore\n").unwrap();
        fixture
            .git_env
            .push(("GIT_CONFIG".into(), "config/legacy.gitconfig".into()));

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());

        for target in [
            legacy,
            included,
            fixture.root().join("legacy.ignore"),
            fixture.root().join("included.ignore"),
        ] {
            assert!(
                policy.dependencies.iter().any(|dependency| {
                    dependency.identity == dependency_path_identity_with_case(&target, true)
                }),
                "missing legacy GIT_CONFIG dependency {}",
                target.display()
            );
        }
    }

    #[test]
    fn missing_and_empty_legacy_git_config_selectors_are_persisted() {
        for (value, target) in [
            (
                OsString::from("config/missing-legacy.gitconfig"),
                PathBuf::from("config/missing-legacy.gitconfig"),
            ),
            (OsString::new(), PathBuf::new()),
        ] {
            let mut fixture = Fixture::new();
            fixture.git_env.push(("GIT_CONFIG".into(), value));

            let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
            let target = lexical_normalize(&fixture.root().join(target));

            assert!(
                policy.dependencies.iter().any(|dependency| {
                    dependency.kind == PolicyDependencyKind::GitConfig
                        && dependency.identity == dependency_path_identity_with_case(&target, true)
                }),
                "missing selected config dependency {}",
                target.display()
            );
        }
    }

    #[test]
    fn every_forwarded_repository_selector_invalidates_direct_reuse() {
        for key in ["GIT_CONFIG", "GIT_DIR", "GIT_COMMON_DIR", "GIT_WORK_TREE"] {
            let mut fixture = Fixture::new();
            let first = fixture.compile(&mut PolicyDependencyMetrics::default());
            fixture.git_env.push((
                key.into(),
                fixture
                    .root()
                    .join(format!("changed-{key}"))
                    .into_os_string(),
            ));

            assert_eq!(
                validate_policy_manifest(&fixture.context(), &first.manifest()).unwrap(),
                PolicyManifestValidation::Changed,
                "selector {key} did not invalidate reuse",
            );
        }
    }

    #[test]
    fn repository_selector_change_forces_full_policy_discovery() {
        let mut fixture = Fixture::new();
        let mut metrics = PolicyDependencyMetrics::default();
        let first = fixture.compile(&mut metrics);
        fixture.git_env.push((
            "GIT_DIR".into(),
            fixture.root().join(".git").into_os_string(),
        ));

        let second = fixture.compile(&mut metrics);

        assert_eq!(metrics.policy_dependency_full_discovery, 2);
        assert!(!second.reused_manifest);
        assert_ne!(first.fingerprint, second.fingerprint);
    }

    #[test]
    fn missing_external_global_config_is_persisted_and_creation_is_detected() {
        let mut fixture = Fixture::new();
        let global = fixture
            .root()
            .parent()
            .unwrap()
            .join("missing-global.gitconfig");
        fixture
            .git_env
            .push(("GIT_CONFIG_GLOBAL".into(), global.as_os_str().to_owned()));

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        let dependency = policy
            .dependencies
            .iter()
            .find(|dependency| dependency.identity == dependency_path_identity(&global))
            .expect("missing global config must remain an authoritative dependency");
        assert!(!dependency.observable);

        fs::write(&global, "[core]\n\texcludesFile = /tmp/excludes\n").unwrap();
        assert_eq!(
            validate_policy_manifest(&fixture.context(), &policy.manifest()).unwrap(),
            PolicyManifestValidation::Changed
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_policy_dependency_is_unobservable_and_mode_identity_is_validated() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let fixture = Fixture::new();
        let external = fixture.root().parent().unwrap().join("external-ignore");
        fs::write(&external, "secret\n").unwrap();
        let nested = fixture.root().join("src/.gitignore");
        fs::remove_file(&nested).unwrap();
        symlink(&external, &nested).unwrap();

        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        let dependency = policy
            .dependencies
            .iter()
            .find(|dependency| dependency.identity == dependency_path_identity(&nested))
            .unwrap();
        assert!(!dependency.observable);
        assert!(policy.stale_baseline);

        fs::remove_file(&nested).unwrap();
        fs::write(&nested, "secret\n").unwrap();
        let mut permissions = fs::metadata(&nested).unwrap().permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&nested, permissions).unwrap();
        assert_eq!(
            validate_policy_manifest(&fixture.context(), &policy.manifest()).unwrap(),
            PolicyManifestValidation::Changed
        );
    }

    #[cfg(unix)]
    #[test]
    fn final_symlink_retarget_and_removal_change_identity_without_target_bytes() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let first_target = fixture.root().parent().unwrap().join("first-secret-ignore");
        let second_target = fixture
            .root()
            .parent()
            .unwrap()
            .join("second-secret-ignore");
        fs::write(&first_target, b"first secret bytes\n").unwrap();
        fs::write(&second_target, b"second secret bytes\n").unwrap();
        let dependency_path = fixture.root().join("final-symlink-ignore");
        symlink(&first_target, &dependency_path).unwrap();
        let (dependency, bytes) = read_file_dependency(
            &dependency_path,
            PolicyDependencyKind::GitExcludesFile,
            1,
            &fixture.context(),
        )
        .unwrap();
        assert!(bytes.is_empty());
        assert_eq!(dependency.content_identity, digest(b""));
        assert_ne!(dependency.content_identity, digest(b"first secret bytes\n"));
        let mut manifest_dependencies = synthetic_dependencies(1, &fixture.context()).unwrap();
        manifest_dependencies.push(dependency.clone());
        manifest_dependencies.sort_by(|left, right| {
            (left.kind, left.identity.as_str()).cmp(&(right.kind, right.identity.as_str()))
        });
        let manifest = PolicyManifest {
            dependencies: manifest_dependencies,
            generation: 1,
            rule_sources: Vec::new(),
        };
        let (_, sources) = validate_policy_manifest_and_pin(&fixture.context(), &manifest).unwrap();
        let source = sources
            .iter()
            .find(|source| source.path == dependency_path)
            .expect("final symlink remains a semantic rule source without target bytes");
        assert!(source.bytes.is_empty());
        fs::remove_file(&dependency_path).unwrap();
        symlink(&second_target, &dependency_path).unwrap();
        let (retargeted, retargeted_bytes) = read_file_dependency(
            &dependency_path,
            PolicyDependencyKind::GitExcludesFile,
            1,
            &fixture.context(),
        )
        .unwrap();
        assert!(retargeted_bytes.is_empty());
        assert_ne!(retargeted.metadata_identity, dependency.metadata_identity);
        fs::remove_file(&dependency_path).unwrap();
        let (removed, removed_bytes) = read_file_dependency(
            &dependency_path,
            PolicyDependencyKind::GitExcludesFile,
            1,
            &fixture.context(),
        )
        .unwrap();
        assert!(removed_bytes.is_empty());
        assert_ne!(removed.metadata_identity, dependency.metadata_identity);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directory_ancestor_never_pins_external_policy_bytes() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let external = fixture.root().parent().unwrap().join("external-config-dir");
        fs::create_dir_all(&external).unwrap();
        fs::write(
            external.join("policy.gitconfig"),
            b"[include]\npath = should-never-be-decoded\n",
        )
        .unwrap();
        let linked_dir = fixture.root().join("linked-config-dir");
        symlink(&external, &linked_dir).unwrap();
        let linked_policy = linked_dir.join("policy.gitconfig");

        let (dependency, bytes) = read_file_dependency(
            &linked_policy,
            PolicyDependencyKind::GitConfig,
            1,
            &fixture.context(),
        )
        .unwrap();

        assert!(bytes.is_empty());
        assert_eq!(dependency.content_identity, digest(b""));
        assert!(!dependency.observable);
    }

    #[cfg(unix)]
    #[test]
    fn ancestor_symlink_retarget_and_removal_change_identity_and_invalidate_raw_events() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let first = fixture
            .root()
            .parent()
            .unwrap()
            .join("first-secret-config-dir");
        let second = fixture
            .root()
            .parent()
            .unwrap()
            .join("second-secret-config-dir");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("policy.gitconfig"), b"first ancestor secret\n").unwrap();
        fs::write(second.join("policy.gitconfig"), b"second ancestor secret\n").unwrap();
        let ancestor = fixture.root().join("linked-policy-dir");
        let dependency_path = ancestor.join("policy.gitconfig");
        symlink(&first, &ancestor).unwrap();
        let (dependency, bytes) = read_file_dependency(
            &dependency_path,
            PolicyDependencyKind::GitConfig,
            1,
            &fixture.context(),
        )
        .unwrap();
        let index = PolicyInvalidationIndex::from_dependencies(
            fixture.root(),
            true,
            std::slice::from_ref(&dependency),
        )
        .unwrap();

        assert!(bytes.is_empty());
        assert_eq!(dependency.content_identity, digest(b""));
        assert_ne!(
            dependency.content_identity,
            digest(b"first ancestor secret\n")
        );
        assert!(index.matches(fixture.root(), &ancestor));
        let policy = finish_compiled_policy(
            &fixture.context(),
            PolicyManifest {
                dependencies: vec![dependency.clone()],
                generation: 1,
                rule_sources: Vec::new(),
            },
            false,
        )
        .unwrap();
        assert!(raw_event_invalidates_policy(&policy, &ancestor));
        fs::remove_file(&ancestor).unwrap();
        symlink(&second, &ancestor).unwrap();
        let (retargeted, retargeted_bytes) = read_file_dependency(
            &dependency_path,
            PolicyDependencyKind::GitConfig,
            1,
            &fixture.context(),
        )
        .unwrap();
        assert!(retargeted_bytes.is_empty());
        assert_ne!(retargeted.metadata_identity, dependency.metadata_identity);
        fs::remove_file(&ancestor).unwrap();
        let (removed, removed_bytes) = read_file_dependency(
            &dependency_path,
            PolicyDependencyKind::GitConfig,
            1,
            &fixture.context(),
        )
        .unwrap();
        assert!(removed_bytes.is_empty());
        assert_ne!(removed.metadata_identity, dependency.metadata_identity);
    }

    #[test]
    fn builtins_normalization_mode_and_case_policy_are_authoritative_identities() {
        let fixture = Fixture::new();
        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        for kind in [
            PolicyDependencyKind::Builtin,
            PolicyDependencyKind::TrailConfig,
            PolicyDependencyKind::Ignore,
            PolicyDependencyKind::Normalization,
            PolicyDependencyKind::Mode,
            PolicyDependencyKind::CasePolicy,
        ] {
            assert!(policy
                .dependencies
                .iter()
                .any(|dependency| dependency.kind == kind));
        }

        let changed_context = PolicyCompileContext {
            case_sensitive: false,
            ..fixture.context()
        };
        assert_eq!(
            validate_policy_manifest(&changed_context, &policy.manifest()).unwrap(),
            PolicyManifestValidation::Changed
        );
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[allow(dead_code)]
    fn path(value: impl Into<PathBuf>) -> PathBuf {
        value.into()
    }
}
use super::ExpectedScope;
use crate::error::{Error, Result};
use crate::model::RecordingConfig;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

const BUILTIN_POLICY_VERSION: &[u8] = b"trail-recording-policy-v1";
const NORMALIZATION_POLICY: &[u8] = b"relative-forward-slash-unicode-nfc-v1";
const MODE_POLICY: &[u8] = b"regular-files-only-no-follow-executable-bit-v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum PolicyDependencyKind {
    Builtin,
    TrailConfig,
    Ignore,
    Trailignore,
    Gitignore,
    GitInfoExclude,
    GitExcludesFile,
    GitConfig,
    Normalization,
    Mode,
    CasePolicy,
}

impl PolicyDependencyKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::TrailConfig => "trail_config",
            Self::Ignore => "ignore",
            Self::Trailignore => "trailignore",
            Self::Gitignore => "gitignore",
            Self::GitInfoExclude => "git_info_exclude",
            Self::GitExcludesFile => "git_excludes_file",
            Self::GitConfig => "git_config",
            Self::Normalization => "normalization",
            Self::Mode => "mode",
            Self::CasePolicy => "case_policy",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "builtin" => Ok(Self::Builtin),
            "trail_config" => Ok(Self::TrailConfig),
            "ignore" => Ok(Self::Ignore),
            "trailignore" => Ok(Self::Trailignore),
            "gitignore" => Ok(Self::Gitignore),
            "git_info_exclude" => Ok(Self::GitInfoExclude),
            "git_excludes_file" => Ok(Self::GitExcludesFile),
            "git_config" => Ok(Self::GitConfig),
            "normalization" => Ok(Self::Normalization),
            "mode" => Ok(Self::Mode),
            "case_policy" => Ok(Self::CasePolicy),
            other => Err(Error::Corrupt(format!(
                "unknown changed-path policy dependency kind `{other}`"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyDependency {
    pub(crate) identity: String,
    pub(crate) kind: PolicyDependencyKind,
    pub(crate) content_identity: [u8; 32],
    pub(crate) metadata_identity: Vec<u8>,
    pub(crate) observable: bool,
    pub(crate) generation: u64,
    pub(crate) last_source_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyManifest {
    pub(crate) dependencies: Vec<PolicyDependency>,
    pub(crate) generation: u64,
    rule_sources: Vec<PolicyRuleSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyRuleSource {
    pub(crate) kind: PolicyDependencyKind,
    pub(crate) path: PathBuf,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordingPolicySnapshot {
    pub(crate) workspace_root: PathBuf,
    pub(crate) ignore_gitignored: bool,
    pub(crate) dependency_files: Vec<PathBuf>,
    pub(crate) case_sensitive: bool,
    pub(crate) rule_sources: Vec<PolicyRuleSource>,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct QualifiedPolicyObserverCut {
    pub(crate) scope_id: super::ScopeId,
    pub(crate) provider_identity: Vec<u8>,
    pub(crate) discovery_started_sequence: u64,
    pub(crate) through_sequence: u64,
    pub(crate) covered_roots: Vec<PathBuf>,
    pub(crate) case_sensitive: bool,
}

#[cfg(test)]
impl QualifiedPolicyObserverCut {
    pub(crate) fn validate_for(
        &self,
        expected: &ExpectedScope,
        workspace_root: &Path,
        case_sensitive: bool,
    ) -> bool {
        self.scope_id == expected.scope_id
            && self.provider_identity == expected.provider_identity
            && self.discovery_started_sequence > 0
            && self.through_sequence >= self.discovery_started_sequence
            && self.case_sensitive == case_sensitive
            && self.covered_roots.iter().any(|root| {
                normalized_path_key(root, case_sensitive)
                    == normalized_path_key(workspace_root, case_sensitive)
            })
    }

    fn covers(&self, path: &Path) -> bool {
        let path = normalized_path_key(path, self.case_sensitive);
        self.covered_roots.iter().any(|root| {
            let root = normalized_path_key(root, self.case_sensitive);
            path == root || path.strip_prefix(&format!("{root}/")).is_some()
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyInvalidationIndex {
    case_sensitive: bool,
    exact_paths: BTreeSet<String>,
}

impl PolicyInvalidationIndex {
    pub(crate) fn from_paths<'a>(
        workspace_root: &Path,
        case_sensitive: bool,
        paths: impl IntoIterator<Item = &'a PathBuf>,
    ) -> Self {
        let exact_paths = paths
            .into_iter()
            .map(|path| {
                let path = if path.is_absolute() {
                    path.clone()
                } else {
                    workspace_root.join(path)
                };
                normalized_path_key(&path, case_sensitive)
            })
            .collect();
        Self {
            case_sensitive,
            exact_paths,
        }
    }

    pub(crate) fn from_dependencies(
        workspace_root: &Path,
        case_sensitive: bool,
        dependencies: &[PolicyDependency],
    ) -> Result<Self> {
        let root = lexical_normalize(workspace_root);
        let mut exact_paths = BTreeSet::new();
        for dependency in dependencies
            .iter()
            .filter(|dependency| dependency_is_file(dependency))
        {
            let path = dependency_identity_path(&dependency.identity)
                .ok_or_else(|| Error::Corrupt("non-canonical policy path identity".into()))?;
            let path = if path.is_absolute() {
                path
            } else {
                root.join(path)
            };
            exact_paths.insert(normalized_path_key(&path, case_sensitive));
            if let Some(unsafe_component) =
                unsafe_component_path_from_metadata_identity(&dependency.metadata_identity)
            {
                exact_paths.insert(normalized_path_key(&unsafe_component, case_sensitive));
            }
        }
        Ok(Self {
            case_sensitive,
            exact_paths,
        })
    }

    pub(crate) fn matches(&self, workspace_root: &Path, path: &Path) -> bool {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace_root.join(path)
        };
        self.exact_paths
            .contains(&normalized_path_key(&path, self.case_sensitive))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdapterEquivalence {
    Equivalent,
    Conservative,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompiledPolicy {
    snapshot: RecordingPolicySnapshot,
    fingerprint: [u8; 32],
    dependencies: Vec<PolicyDependency>,
    adapter_equivalence: AdapterEquivalence,
    stale_baseline: bool,
    reused_manifest: bool,
    invalidation_index: PolicyInvalidationIndex,
    reconciliation_authorization: Option<PolicyReconciliationAuthorization>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PolicyReconciliationAuthorization {
    expected: ExpectedScope,
}

impl CompiledPolicy {
    pub(crate) fn manifest(&self) -> PolicyManifest {
        PolicyManifest {
            generation: self
                .dependencies
                .first()
                .map(|dependency| dependency.generation)
                .unwrap_or(0),
            dependencies: self.dependencies.clone(),
            rule_sources: self.snapshot.rule_sources.clone(),
        }
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        &self.snapshot.workspace_root
    }

    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }

    pub(crate) fn dependency_files(&self) -> &[PathBuf] {
        &self.snapshot.dependency_files
    }

    pub(crate) fn authorizes_reconciliation(&self, expected: &ExpectedScope) -> bool {
        self.adapter_equivalence == AdapterEquivalence::Equivalent
            && !self.stale_baseline
            && self
                .reconciliation_authorization
                .as_ref()
                .is_some_and(|authorization| authorization.expected == *expected)
    }

    pub(crate) fn authorize_native_reconciliation(
        &mut self,
        expected: &ExpectedScope,
        lease: &super::ObserverLease,
    ) -> Result<()> {
        if self.fingerprint != expected.policy_fingerprint
            || lease.root_identity != expected.filesystem_identity
            || lease.provider_identity != expected.provider_identity
            || lease.policy_dependencies != self.snapshot.dependency_files
            || !lease.capabilities.durable_cursor
            || !lease.capabilities.linearizable_fence
            || !lease.capabilities.overflow_scope
            || !lease.capabilities.filesystem_supported
            || !lease.capabilities.clean_proof_allowed
            || !lease.capabilities.power_loss_durability
        {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: expected.scope_id.to_text(),
                state: super::TrustState::StaleBaseline.as_str().into(),
                reason: "native observer lease cannot authorize the compiled recording policy"
                    .into(),
                command: "trail status".into(),
            });
        }
        self.adapter_equivalence = AdapterEquivalence::Equivalent;
        self.stale_baseline = false;
        self.reconciliation_authorization = Some(PolicyReconciliationAuthorization {
            expected: expected.clone(),
        });
        Ok(())
    }

    #[cfg(any(test, debug_assertions))]
    pub(crate) fn authorize_reconciliation_for_test(&mut self, expected: &ExpectedScope) {
        self.adapter_equivalence = AdapterEquivalence::Equivalent;
        self.stale_baseline = false;
        self.reconciliation_authorization = Some(PolicyReconciliationAuthorization {
            expected: expected.clone(),
        });
    }

    #[cfg(test)]
    pub(crate) fn set_ignore_gitignored_for_test(&mut self, ignore_gitignored: bool) {
        self.snapshot.ignore_gitignored = ignore_gitignored;
    }

    #[cfg(test)]
    pub(crate) fn set_gitignore_rule_for_test(&mut self, path: PathBuf, bytes: Vec<u8>) {
        self.snapshot.rule_sources = vec![PolicyRuleSource {
            kind: PolicyDependencyKind::Gitignore,
            path,
            bytes,
        }];
    }

    #[cfg(any(test, debug_assertions))]
    pub(crate) fn for_reconciliation_test(
        snapshot: RecordingPolicySnapshot,
        fingerprint: [u8; 32],
        expected: &ExpectedScope,
    ) -> Self {
        let invalidation_index = PolicyInvalidationIndex::from_paths(
            &snapshot.workspace_root,
            snapshot.case_sensitive,
            snapshot.dependency_files.iter(),
        );
        let mut policy = Self {
            snapshot,
            fingerprint,
            dependencies: Vec::new(),
            adapter_equivalence: AdapterEquivalence::Conservative,
            stale_baseline: true,
            reused_manifest: false,
            invalidation_index,
            reconciliation_authorization: None,
        };
        policy.authorize_reconciliation_for_test(expected);
        policy
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PolicyManifestValidation {
    Current,
    Changed,
    Unobservable,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PolicyDependencyMetrics {
    pub(crate) policy_dependency_full_discovery: u64,
    pub(crate) policy_dependency_direct_checks: u64,
}

pub(crate) struct PolicyCompileContext<'a> {
    pub(crate) workspace_root: &'a Path,
    pub(crate) db_dir: &'a Path,
    pub(crate) recording: &'a RecordingConfig,
    pub(crate) case_sensitive: bool,
    pub(crate) git_environment: &'a [(OsString, OsString)],
}

pub(crate) fn compile_policy(
    conn: &Connection,
    expected: &ExpectedScope,
    context: &PolicyCompileContext<'_>,
    metrics: &mut PolicyDependencyMetrics,
) -> Result<CompiledPolicy> {
    conn.execute_batch("SAVEPOINT changed_path_policy_compile;")?;
    let result = compile_policy_guarded(conn, expected, context, metrics);
    match result {
        Ok(policy) => {
            conn.execute_batch("RELEASE changed_path_policy_compile;")?;
            Ok(policy)
        }
        Err(err) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO changed_path_policy_compile; RELEASE changed_path_policy_compile;",
            );
            Err(err)
        }
    }
}

fn compile_policy_guarded(
    conn: &Connection,
    expected: &ExpectedScope,
    context: &PolicyCompileContext<'_>,
    metrics: &mut PolicyDependencyMetrics,
) -> Result<CompiledPolicy> {
    policy_scope_cas_guard(conn, expected)?;
    let stored = load_policy_manifest(conn, expected)?;
    if let Some(mut manifest) = stored {
        metrics.policy_dependency_direct_checks = metrics
            .policy_dependency_direct_checks
            .saturating_add(manifest.dependencies.len() as u64);
        let (validation, rule_sources) = validate_policy_manifest_and_pin(context, &manifest)?;
        manifest.rule_sources = rule_sources;
        match validation {
            PolicyManifestValidation::Current => {
                let policy = finish_compiled_policy(context, manifest, true)?;
                mark_policy_stale_guarded(conn, expected, "policy_observer_cut_unavailable")?;
                return Ok(policy);
            }
            PolicyManifestValidation::Unobservable => {
                let policy = finish_compiled_policy(context, manifest, true)?;
                mark_policy_stale_guarded(conn, expected, "policy_observer_cut_unavailable")?;
                return Ok(policy);
            }
            PolicyManifestValidation::Changed => {}
        }
    }

    metrics.policy_dependency_full_discovery =
        metrics.policy_dependency_full_discovery.saturating_add(1);
    let mut manifest = discover_policy_manifest(expected.policy_generation, context)?;
    let (validation, rule_sources) = validate_policy_manifest_and_pin(context, &manifest)?;
    if validation == PolicyManifestValidation::Changed {
        return Err(Error::InvalidInput(
            "policy dependency changed during discovery; retry reconciliation".into(),
        ));
    }
    manifest.rule_sources = rule_sources;
    persist_policy_manifest_rows(conn, expected, &manifest)?;
    let policy = finish_compiled_policy(context, manifest, false)?;
    mark_policy_stale_guarded(conn, expected, "policy_observer_cut_unavailable")?;
    Ok(policy)
}

fn finish_compiled_policy(
    context: &PolicyCompileContext<'_>,
    manifest: PolicyManifest,
    reused_manifest: bool,
) -> Result<CompiledPolicy> {
    let fingerprint = policy_fingerprint(&manifest.dependencies)?;
    let mut dependency_files = manifest
        .dependencies
        .iter()
        .filter(|dependency| dependency_is_file(dependency))
        .filter_map(|dependency| dependency_identity_path(&dependency.identity))
        .collect::<Vec<_>>();
    dependency_files.sort();
    dependency_files.dedup();
    let invalidation_index = PolicyInvalidationIndex::from_dependencies(
        context.workspace_root,
        context.case_sensitive,
        &manifest.dependencies,
    )?;
    Ok(CompiledPolicy {
        snapshot: RecordingPolicySnapshot {
            workspace_root: context.workspace_root.to_path_buf(),
            ignore_gitignored: context.recording.ignore_gitignored,
            dependency_files,
            case_sensitive: context.case_sensitive,
            rule_sources: manifest.rule_sources.clone(),
        },
        fingerprint,
        dependencies: manifest.dependencies,
        // Task 4 persists dependency evidence but has no authorized trust
        // promotion. Even a future crate-local observer proof cannot enter
        // this compile context or change the result.
        adapter_equivalence: AdapterEquivalence::Conservative,
        stale_baseline: true,
        reused_manifest,
        invalidation_index,
        reconciliation_authorization: None,
    })
}

pub(crate) fn validate_policy_manifest(
    context: &PolicyCompileContext<'_>,
    manifest: &PolicyManifest,
) -> Result<PolicyManifestValidation> {
    validate_policy_manifest_and_pin(context, manifest).map(|(validation, _)| validation)
}

fn validate_policy_manifest_and_pin(
    context: &PolicyCompileContext<'_>,
    manifest: &PolicyManifest,
) -> Result<(PolicyManifestValidation, Vec<PolicyRuleSource>)> {
    validate_manifest_canonical(manifest)?;
    let synthetic = synthetic_dependencies(manifest.generation, context)?
        .into_iter()
        .map(|dependency| ((dependency.kind, dependency.identity.clone()), dependency))
        .collect::<BTreeMap<_, _>>();
    let mut unobservable = false;
    let mut rule_sources = Vec::new();
    for dependency in &manifest.dependencies {
        let (current, bytes) = if dependency_is_file(dependency) {
            let Some(path) = dependency_identity_path(&dependency.identity) else {
                return Ok((PolicyManifestValidation::Changed, Vec::new()));
            };
            read_file_dependency(&path, dependency.kind, dependency.generation, context)?
        } else {
            let Some(current) = synthetic.get(&(dependency.kind, dependency.identity.clone()))
            else {
                return Ok((PolicyManifestValidation::Changed, Vec::new()));
            };
            (current.clone(), Vec::new())
        };
        if dependency.content_identity != current.content_identity
            || dependency.metadata_identity != current.metadata_identity
            || dependency.observable != current.observable
            || dependency.generation != current.generation
        {
            return Ok((PolicyManifestValidation::Changed, Vec::new()));
        }
        if dependency_is_rule_file(dependency.kind) {
            let path = dependency_identity_path(&dependency.identity)
                .ok_or_else(|| Error::Corrupt("invalid policy rule identity".into()))?;
            rule_sources.push(PolicyRuleSource {
                kind: dependency.kind,
                path,
                bytes,
            });
        }
        unobservable |= !dependency.observable;
    }
    if manifest
        .dependencies
        .iter()
        .filter(|dependency| !dependency_is_file(dependency))
        .count()
        != synthetic.len()
    {
        return Ok((PolicyManifestValidation::Changed, Vec::new()));
    }
    Ok((
        if unobservable {
            PolicyManifestValidation::Unobservable
        } else {
            PolicyManifestValidation::Current
        },
        rule_sources,
    ))
}

fn dependency_is_file(dependency: &PolicyDependency) -> bool {
    dependency.identity.starts_with("path:")
}

fn dependency_is_rule_file(kind: PolicyDependencyKind) -> bool {
    matches!(
        kind,
        PolicyDependencyKind::Ignore
            | PolicyDependencyKind::Trailignore
            | PolicyDependencyKind::Gitignore
            | PolicyDependencyKind::GitInfoExclude
            | PolicyDependencyKind::GitExcludesFile
    )
}

pub(crate) fn raw_event_invalidates_policy(policy: &CompiledPolicy, path: &Path) -> bool {
    raw_path_may_invalidate_policy_with_case(path, policy.snapshot.case_sensitive)
        || policy
            .invalidation_index
            .matches(&policy.snapshot.workspace_root, path)
}

pub(crate) fn raw_path_may_invalidate_policy(path: &Path) -> bool {
    raw_path_may_invalidate_policy_with_case(path, platform_default_case_sensitive())
}

fn raw_path_may_invalidate_policy_with_case(path: &Path, case_sensitive: bool) -> bool {
    let mut normalized = path.to_string_lossy().replace('\\', "/");
    if !case_sensitive {
        normalized = normalized.to_lowercase();
    }
    let file_name = normalized.rsplit('/').next();
    matches!(file_name, Some(".ignore" | ".trailignore" | ".gitignore"))
        || normalized.ends_with("/.trail/config.toml")
        || normalized == ".trail/config.toml"
        || normalized.ends_with("/.git/info/exclude")
        || normalized == ".git/info/exclude"
        || normalized.ends_with("/.git/config")
        || normalized == ".git/config"
        || normalized.ends_with("/.git/config.worktree")
        || normalized == ".git/config.worktree"
}

fn discover_policy_manifest(
    generation: u64,
    context: &PolicyCompileContext<'_>,
) -> Result<PolicyManifest> {
    let mut dependencies = synthetic_dependencies(generation, context)?;
    let trail_config = lexical_normalize(&context.db_dir.join("config.toml"));
    dependencies.push(file_dependency(
        &trail_config,
        PolicyDependencyKind::TrailConfig,
        generation,
        context,
    )?);

    let root = lexical_normalize(context.workspace_root);
    for (name, kind) in [
        (".ignore", PolicyDependencyKind::Ignore),
        (".trailignore", PolicyDependencyKind::Trailignore),
        (".gitignore", PolicyDependencyKind::Gitignore),
    ] {
        dependencies.push(file_dependency(
            &root.join(name),
            kind,
            generation,
            context,
        )?);
    }
    let walker = WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let Ok(relative) = entry.path().strip_prefix(&root) else {
                return false;
            };
            let first = relative.components().next();
            !matches!(
                first,
                Some(Component::Normal(name)) if name == OsStr::new(".git") || name == OsStr::new(".trail")
            )
        });
    for entry in walker {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if entry.depth() == 1 {
            continue;
        }
        if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
            continue;
        }
        let kind = match entry.file_name().to_str() {
            Some(".ignore") => PolicyDependencyKind::Ignore,
            Some(".trailignore") => PolicyDependencyKind::Trailignore,
            Some(".gitignore") => PolicyDependencyKind::Gitignore,
            _ => continue,
        };
        dependencies.push(file_dependency(entry.path(), kind, generation, context)?);
    }

    dependencies.extend(discover_git_dependencies(generation, context)?);
    dependencies.sort_by(|left, right| {
        (left.kind, left.identity.as_str()).cmp(&(right.kind, right.identity.as_str()))
    });
    let manifest = PolicyManifest {
        dependencies,
        generation,
        rule_sources: Vec::new(),
    };
    validate_manifest_canonical(&manifest)?;
    Ok(manifest)
}

fn synthetic_dependencies(
    generation: u64,
    context: &PolicyCompileContext<'_>,
) -> Result<Vec<PolicyDependency>> {
    let recording = serde_json::to_vec(context.recording)
        .map_err(|err| Error::InvalidInput(err.to_string()))?;
    let mut dependencies = vec![
        synthetic_dependency(
            "builtin:recording-policy",
            PolicyDependencyKind::Builtin,
            BUILTIN_POLICY_VERSION,
            generation,
        ),
        synthetic_dependency(
            "trail-config:recording",
            PolicyDependencyKind::TrailConfig,
            &recording,
            generation,
        ),
        synthetic_dependency(
            "normalization:path",
            PolicyDependencyKind::Normalization,
            NORMALIZATION_POLICY,
            generation,
        ),
        synthetic_dependency(
            "mode:filesystem-entry",
            PolicyDependencyKind::Mode,
            MODE_POLICY,
            generation,
        ),
        synthetic_dependency(
            "case-policy:scope",
            PolicyDependencyKind::CasePolicy,
            if context.case_sensitive {
                b"case-sensitive-v1"
            } else {
                b"case-insensitive-v1"
            },
            generation,
        ),
    ];
    let mut selector_keys = [
        OsString::from("HOME"),
        OsString::from("XDG_CONFIG_HOME"),
        OsString::from("GIT_CONFIG"),
        OsString::from("GIT_DIR"),
        OsString::from("GIT_COMMON_DIR"),
        OsString::from("GIT_WORK_TREE"),
        OsString::from("GIT_CONFIG_GLOBAL"),
        OsString::from("GIT_CONFIG_SYSTEM"),
        OsString::from("GIT_CONFIG_NOSYSTEM"),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    selector_keys.extend(
        context
            .git_environment
            .iter()
            .map(|(key, _)| key.clone())
            .filter(|key| key.to_string_lossy().starts_with("GIT_")),
    );
    for key in selector_keys {
        let value = git_environment_value_os(context, &key)
            .map(|value| os_str_bytes(&value).to_vec())
            .unwrap_or_else(|| b"<unset>".to_vec());
        dependencies.push(synthetic_dependency(
            &format!("git-env:{}", hex::encode(os_str_bytes(&key))),
            PolicyDependencyKind::GitConfig,
            &value,
            generation,
        ));
    }
    Ok(dependencies)
}

fn synthetic_dependency(
    identity: &str,
    kind: PolicyDependencyKind,
    content: &[u8],
    generation: u64,
) -> PolicyDependency {
    PolicyDependency {
        identity: identity.to_string(),
        kind,
        content_identity: digest(content),
        metadata_identity: b"synthetic-v1".to_vec(),
        observable: true,
        generation,
        last_source_sequence: 0,
    }
}

fn discover_git_dependencies(
    generation: u64,
    context: &PolicyCompileContext<'_>,
) -> Result<Vec<PolicyDependency>> {
    if !context.recording.ignore_gitignored {
        return Ok(Vec::new());
    }
    let Some(git_dirs) = run_git_optional_repository(
        context,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
    )?
    else {
        return Ok(Vec::new());
    };
    let mut paths = BTreeMap::<(PolicyDependencyKind, String), PathBuf>::new();
    let cwd = lexical_normalize(context.workspace_root);
    let home = git_environment_value(context, "HOME").map(|value| {
        if value.is_empty() {
            PathBuf::from("/")
        } else {
            resolve_git_cwd_path(&cwd, PathBuf::from(value))
        }
    });
    let xdg = git_environment_value(context, "XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| resolve_git_cwd_path(&cwd, path))
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    if let Some(selected) = git_environment_value(context, "GIT_CONFIG") {
        insert_git_dependency_path(
            &mut paths,
            context.case_sensitive,
            resolve_git_cwd_path(&cwd, PathBuf::from(selected)),
            PolicyDependencyKind::GitConfig,
        );
    }
    if let Some(global) = git_environment_value(context, "GIT_CONFIG_GLOBAL") {
        if !global.is_empty() {
            insert_git_dependency_path(
                &mut paths,
                context.case_sensitive,
                resolve_git_cwd_path(&cwd, PathBuf::from(global)),
                PolicyDependencyKind::GitConfig,
            );
        }
    } else {
        if let Some(home) = &home {
            insert_git_dependency_path(
                &mut paths,
                context.case_sensitive,
                home.join(".gitconfig"),
                PolicyDependencyKind::GitConfig,
            );
        }
        if let Some(xdg) = &xdg {
            insert_git_dependency_path(
                &mut paths,
                context.case_sensitive,
                xdg.join("git/config"),
                PolicyDependencyKind::GitConfig,
            );
        }
    }
    if !git_environment_value(context, "GIT_CONFIG_NOSYSTEM").is_some_and(|v| git_truthy(&v)) {
        let system = git_environment_value(context, "GIT_CONFIG_SYSTEM")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/gitconfig"));
        if !system.as_os_str().is_empty() {
            insert_git_dependency_path(
                &mut paths,
                context.case_sensitive,
                resolve_git_cwd_path(&cwd, system),
                PolicyDependencyKind::GitConfig,
            );
        }
    }
    if let Some(xdg) = &xdg {
        insert_git_dependency_path(
            &mut paths,
            context.case_sensitive,
            xdg.join("git/ignore"),
            PolicyDependencyKind::GitExcludesFile,
        );
    }
    let mut lines = git_dirs.split(|byte| *byte == b'\n');
    let git_dir = lines
        .next()
        .filter(|line| !line.is_empty())
        .map(|line| PathBuf::from(os_string_from_bytes(line)));
    if let Some(git_dir) = &git_dir {
        insert_git_dependency_path(
            &mut paths,
            context.case_sensitive,
            git_dir.join("config"),
            PolicyDependencyKind::GitConfig,
        );
        insert_git_dependency_path(
            &mut paths,
            context.case_sensitive,
            git_dir.join("config.worktree"),
            PolicyDependencyKind::GitConfig,
        );
    }
    if let Some(common) = lines.next().filter(|line| !line.is_empty()) {
        insert_git_dependency_path(
            &mut paths,
            context.case_sensitive,
            PathBuf::from(os_string_from_bytes(common)).join("info/exclude"),
            PolicyDependencyKind::GitInfoExclude,
        );
    }

    for entry in injected_git_config_entries(context)? {
        if git_config_key_is_include(&entry.key) {
            if let Some(path) = resolve_git_include_path(&entry.value, &cwd, home.as_deref()) {
                insert_git_dependency_path(
                    &mut paths,
                    context.case_sensitive,
                    path,
                    PolicyDependencyKind::GitConfig,
                );
            }
        } else if entry.key.eq_ignore_ascii_case(b"core.excludesfile") {
            if let Some(path) = resolve_git_config_path(&entry.value, &cwd, home.as_deref()) {
                insert_git_dependency_path(
                    &mut paths,
                    context.case_sensitive,
                    path,
                    PolicyDependencyKind::GitExcludesFile,
                );
            }
        }
    }

    // Ask Git to decode each safely pinned config independently. Keeping
    // includes disabled makes missing and inactive include/includeIf targets
    // visible without allowing Git to reopen or recursively follow paths.
    let mut pending = paths
        .iter()
        .filter(|((kind, _), _)| *kind == PolicyDependencyKind::GitConfig)
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
    let mut parsed = BTreeSet::new();
    while let Some(config) = pending.pop() {
        let key = normalized_path_key(&config, context.case_sensitive);
        if !parsed.insert(key) {
            continue;
        }
        let Some(bytes) = read_path_bytes_no_follow(&config)? else {
            continue;
        };
        for entry in git_config_entries_from_bytes(context, &bytes)? {
            if git_config_key_is_include(&entry.key) {
                if let Some(included) = resolve_git_include_path(
                    &entry.value,
                    config.parent().unwrap_or(Path::new("/")),
                    home.as_deref(),
                ) {
                    let included_key = (
                        PolicyDependencyKind::GitConfig,
                        normalized_path_key(&included, context.case_sensitive),
                    );
                    if !paths.contains_key(&included_key) {
                        paths.insert(included_key, included.clone());
                        pending.push(included);
                    }
                }
            } else if entry.key.eq_ignore_ascii_case(b"core.excludesfile") {
                if let Some(path) = resolve_git_config_path(&entry.value, &cwd, home.as_deref()) {
                    insert_git_dependency_path(
                        &mut paths,
                        context.case_sensitive,
                        path,
                        PolicyDependencyKind::GitExcludesFile,
                    );
                }
            }
        }
    }

    paths
        .into_iter()
        .map(|((kind, _), path)| file_dependency(&path, kind, generation, context))
        .collect()
}

fn insert_git_dependency_path(
    paths: &mut BTreeMap<(PolicyDependencyKind, String), PathBuf>,
    case_sensitive: bool,
    path: PathBuf,
    kind: PolicyDependencyKind,
) {
    let path = lexical_normalize(&path);
    paths.insert((kind, normalized_path_key(&path, case_sensitive)), path);
}

#[derive(Debug)]
struct GitConfigEntry {
    key: Vec<u8>,
    value: OsString,
}

const GIT_POLICY_KEY_PATTERN: &str = r"^(include\.path|include[Ii]f\..*\.path|core\.excludesfile)$";

fn git_config_entries_from_bytes(
    context: &PolicyCompileContext<'_>,
    bytes: &[u8],
) -> Result<Vec<GitConfigEntry>> {
    let output = run_git_with_stdin(
        context,
        &[
            "config",
            "--file",
            "-",
            "--no-includes",
            "--show-origin",
            "--null",
            "--get-regexp",
            GIT_POLICY_KEY_PATTERN,
        ],
        false,
        bytes,
    )?;
    expand_git_config_entry_paths(context, parse_git_config_entries(&output, false)?)
}

fn injected_git_config_entries(context: &PolicyCompileContext<'_>) -> Result<Vec<GitConfigEntry>> {
    let output = run_injected_git_config(context)?;
    expand_git_config_entry_paths(context, parse_git_config_entries(&output, true)?)
}

fn expand_git_config_entry_paths(
    context: &PolicyCompileContext<'_>,
    entries: Vec<GitConfigEntry>,
) -> Result<Vec<GitConfigEntry>> {
    entries
        .into_iter()
        .map(|entry| {
            Ok(GitConfigEntry {
                key: entry.key,
                value: git_expand_path_value(context, &entry.value)?,
            })
        })
        .collect()
}

fn parse_git_config_entries(output: &[u8], with_scope: bool) -> Result<Vec<GitConfigEntry>> {
    let fields = output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    let width = if with_scope { 3 } else { 2 };
    if fields.len() % width != 0 {
        return Err(Error::InvalidInput(
            "git config returned malformed origin output".into(),
        ));
    }
    let mut entries = Vec::new();
    for record in fields.chunks_exact(width) {
        if with_scope && record[0] != b"command" {
            continue;
        }
        let key_value = record[width - 1];
        let Some(separator) = key_value.iter().position(|byte| *byte == b'\n') else {
            return Err(Error::InvalidInput(
                "git config returned an entry without a key/value separator".into(),
            ));
        };
        entries.push(GitConfigEntry {
            key: key_value[..separator].to_vec(),
            value: os_string_from_bytes(&key_value[separator + 1..]),
        });
    }
    Ok(entries)
}

fn git_config_key_is_include(key: &[u8]) -> bool {
    let key = String::from_utf8_lossy(key).to_ascii_lowercase();
    key == "include.path" || (key.starts_with("includeif.") && key.ends_with(".path"))
}

fn normalize_git_config_count(value: &OsStr) -> OsString {
    let bytes = os_str_bytes(value);
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|index| index + 1)
        .unwrap_or(start);
    if start == end {
        OsString::from("0")
    } else {
        os_string_from_bytes(&bytes[start..end])
    }
}

fn resolve_git_cwd_path(cwd: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        lexical_normalize(&path)
    } else {
        lexical_normalize(&cwd.join(path))
    }
}

fn resolve_git_include_path(value: &OsStr, base: &Path, _home: Option<&Path>) -> Option<PathBuf> {
    resolve_git_config_path(value, base, None)
}

fn resolve_git_config_path(value: &OsStr, cwd: &Path, _home: Option<&Path>) -> Option<PathBuf> {
    if value.is_empty() {
        return None;
    }
    Some(resolve_git_cwd_path(cwd, PathBuf::from(value)))
}

fn git_truthy(value: &OsStr) -> bool {
    !matches!(
        value.to_string_lossy().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "off"
    )
}

fn git_environment_value(context: &PolicyCompileContext<'_>, key: &str) -> Option<OsString> {
    git_environment_value_os(context, OsStr::new(key))
}

fn git_environment_value_os(context: &PolicyCompileContext<'_>, key: &OsStr) -> Option<OsString> {
    context
        .git_environment
        .iter()
        .rev()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.clone())
}

fn git_command_environment(
    context: &PolicyCompileContext<'_>,
    ambient: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let mut environment = BTreeMap::new();
    for (key, value) in ambient {
        if key == OsStr::new("PATH") {
            environment.insert(key, value);
        }
    }
    for (key, value) in context.git_environment {
        environment.insert(
            key.clone(),
            if key == OsStr::new("GIT_CONFIG_COUNT") {
                normalize_git_config_count(value)
            } else {
                value.clone()
            },
        );
    }
    environment.into_iter().collect()
}

fn git_expand_path_value(context: &PolicyCompileContext<'_>, value: &OsStr) -> Result<OsString> {
    let mut assignment = OsString::from("trail.policyPath=");
    assignment.push(value);
    let ambient = git_command_environment(context, std::env::vars_os());
    let mut environment = BTreeMap::new();
    for (key, value) in ambient {
        if key == OsStr::new("PATH") || key == OsStr::new("HOME") {
            environment.insert(key, value);
        }
    }
    environment.insert(
        OsString::from("XDG_CONFIG_HOME"),
        OsString::from("/dev/null"),
    );
    environment.insert(
        OsString::from("GIT_CONFIG_GLOBAL"),
        OsString::from("/dev/null"),
    );
    environment.insert(OsString::from("GIT_CONFIG_NOSYSTEM"), OsString::from("1"));
    let output = Command::new("git")
        .arg("-c")
        .arg(assignment)
        .args(["config", "--path", "--get", "trail.policyPath"])
        .current_dir(context.workspace_root)
        .env_clear()
        .envs(environment)
        .output()
        .map_err(Error::Io)?;
    let output = git_output(
        &["-c", "trail.policyPath=<value>", "config", "--path"],
        true,
        output,
    )?;
    Ok(os_string_from_bytes(
        output.strip_suffix(b"\n").unwrap_or(&output),
    ))
}

fn run_git(context: &PolicyCompileContext<'_>, args: &[&str], required: bool) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command.args(args).current_dir(context.workspace_root);
    command
        .env_clear()
        .envs(git_command_environment(context, std::env::vars_os()));
    let output = command.output().map_err(Error::Io)?;
    git_output(args, required, output)
}

fn run_git_optional_repository(
    context: &PolicyCompileContext<'_>,
    args: &[&str],
) -> Result<Option<Vec<u8>>> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(context.workspace_root)
        .env_clear()
        .envs(git_command_environment(context, std::env::vars_os()))
        .env("LC_ALL", "C");
    let output = command.output().map_err(Error::Io)?;
    if output.status.success() {
        return Ok(Some(output.stdout));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(128) && stderr.contains("not a git repository") {
        return Ok(None);
    }
    git_output(args, true, output).map(Some)
}

fn run_git_with_stdin(
    context: &PolicyCompileContext<'_>,
    args: &[&str],
    required: bool,
    stdin: &[u8],
) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(context.workspace_root)
        .env_clear()
        .envs(git_command_environment(context, std::env::vars_os()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(Error::Io)?;
    child
        .stdin
        .take()
        .ok_or_else(|| Error::InvalidInput("git config stdin was unavailable".into()))?
        .write_all(stdin)
        .map_err(Error::Io)?;
    let output = child.wait_with_output().map_err(Error::Io)?;
    git_output(args, required, output)
}

fn run_injected_git_config(context: &PolicyCompileContext<'_>) -> Result<Vec<u8>> {
    let args = [
        "config",
        "--no-includes",
        "--show-origin",
        "--show-scope",
        "--null",
        "--get-regexp",
        GIT_POLICY_KEY_PATTERN,
    ];
    let ambient = git_command_environment(context, std::env::vars_os());
    let mut environment = BTreeMap::new();
    for (key, value) in ambient {
        if key == OsStr::new("PATH")
            || key == OsStr::new("GIT_CONFIG_PARAMETERS")
            || key == OsStr::new("GIT_CONFIG_COUNT")
            || key.to_string_lossy().starts_with("GIT_CONFIG_KEY_")
            || key.to_string_lossy().starts_with("GIT_CONFIG_VALUE_")
        {
            environment.insert(key, value);
        }
    }
    environment.insert(OsString::from("HOME"), OsString::from("/"));
    environment.insert(
        OsString::from("XDG_CONFIG_HOME"),
        OsString::from("/dev/null"),
    );
    environment.insert(
        OsString::from("GIT_CONFIG_GLOBAL"),
        OsString::from("/dev/null"),
    );
    environment.insert(OsString::from("GIT_CONFIG_NOSYSTEM"), OsString::from("1"));

    let output = Command::new("git")
        .args(args)
        .current_dir("/")
        .env_clear()
        .envs(environment)
        .output()
        .map_err(Error::Io)?;
    git_output(&args, false, output)
}

fn git_output(args: &[&str], required: bool, output: std::process::Output) -> Result<Vec<u8>> {
    if output.status.success() {
        return Ok(output.stdout);
    }
    if !required && output.status.code() == Some(1) {
        return Ok(Vec::new());
    }
    Err(Error::InvalidInput(format!(
        "git {} failed while compiling recording policy: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

fn file_dependency(
    path: &Path,
    kind: PolicyDependencyKind,
    generation: u64,
    context: &PolicyCompileContext<'_>,
) -> Result<PolicyDependency> {
    read_file_dependency(path, kind, generation, context).map(|(dependency, _)| dependency)
}

fn read_file_dependency(
    path: &Path,
    kind: PolicyDependencyKind,
    generation: u64,
    _context: &PolicyCompileContext<'_>,
) -> Result<(PolicyDependency, Vec<u8>)> {
    let path = lexical_normalize(path);
    let state = read_file_state_no_follow(&path)?;
    // Task 4 has no native-observer handoff yet. Direct filesystem checks can
    // validate reuse, but they cannot establish observer continuity, so file
    // dependencies remain explicitly uncovered until a later task wires a
    // qualified discovery cut into this compiler.
    let observable = false;
    let dependency = PolicyDependency {
        identity: dependency_path_identity(&path),
        kind,
        content_identity: digest(&state.bytes),
        metadata_identity: match state.unsafe_metadata_identity {
            Some(identity) => identity,
            None => metadata_identity(state.metadata.as_ref(), &path)?,
        },
        observable,
        generation,
        last_source_sequence: 0,
    };
    Ok((dependency, state.bytes))
}

fn read_path_bytes_no_follow(path: &Path) -> Result<Option<Vec<u8>>> {
    let state = read_file_state_no_follow(path)?;
    Ok(state
        .metadata
        .filter(|metadata| metadata.is_file())
        .map(|_| state.bytes))
}

struct NoFollowFileState {
    metadata: Option<fs::Metadata>,
    bytes: Vec<u8>,
    unsafe_metadata_identity: Option<Vec<u8>>,
}

impl NoFollowFileState {
    fn missing() -> Self {
        Self {
            metadata: None,
            bytes: Vec::new(),
            unsafe_metadata_identity: None,
        }
    }

    fn unsafe_component(metadata_identity: Vec<u8>) -> Self {
        Self {
            metadata: None,
            bytes: Vec::new(),
            unsafe_metadata_identity: Some(metadata_identity),
        }
    }
}

fn read_file_state_no_follow(path: &Path) -> Result<NoFollowFileState> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        return read_file_state_openat(path);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
        Ok(NoFollowFileState::missing())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_file_state_openat(path: &Path) -> Result<NoFollowFileState> {
    use rustix::fs::{openat, Mode, OFlags, CWD};

    let path = lexical_normalize(path);
    if !path.is_absolute() {
        return Err(Error::InvalidInput(format!(
            "policy dependency `{}` is not absolute",
            path.display()
        )));
    }
    let components = path
        .strip_prefix(Path::new("/"))
        .ok()
        .map(|relative| relative.components().collect::<Vec<_>>())
        .unwrap_or_default();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::InvalidInput(format!(
            "policy dependency `{}` cannot be traversed safely",
            path.display()
        )));
    }

    for _ in 0..2 {
        let mut directory = match openat(
            CWD,
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(directory) => directory,
            Err(err) => return Err(Error::Io(err.into())),
        };
        let mut traversed = PathBuf::from("/");
        for component in &components[..components.len() - 1] {
            let Component::Normal(name) = component else {
                return Err(Error::InvalidInput(format!(
                    "policy dependency `{}` cannot be traversed safely",
                    path.display()
                )));
            };
            traversed.push(name);
            directory = match openat(
                &directory,
                Path::new(name),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(directory) => directory,
                Err(open_error) => {
                    return unsafe_open_failure_state(
                        &directory,
                        Path::new(name),
                        &traversed,
                        open_error,
                    )
                }
            };
        }
        let Component::Normal(file_name) = components[components.len() - 1] else {
            return Err(Error::InvalidInput(format!(
                "policy dependency `{}` cannot be traversed safely",
                path.display()
            )));
        };
        let descriptor = match openat(
            &directory,
            Path::new(file_name),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(descriptor) => descriptor,
            Err(open_error) => {
                return unsafe_open_failure_state(
                    &directory,
                    Path::new(file_name),
                    &path,
                    open_error,
                )
            }
        };
        let mut file = fs::File::from(descriptor);
        let before = file.metadata().map_err(Error::Io)?;
        if !before.is_file() {
            return Ok(NoFollowFileState {
                metadata: Some(before),
                bytes: Vec::new(),
                unsafe_metadata_identity: None,
            });
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(Error::Io)?;
        let after = file.metadata().map_err(Error::Io)?;
        if metadata_identity(Some(&before), &path)? == metadata_identity(Some(&after), &path)? {
            return Ok(NoFollowFileState {
                metadata: Some(after),
                bytes,
                unsafe_metadata_identity: None,
            });
        }
    }
    Err(Error::InvalidInput(format!(
        "policy dependency `{}` changed while it was read",
        path.display()
    )))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unsafe_open_failure_state<Fd: std::os::fd::AsFd>(
    directory: Fd,
    name: &Path,
    component_path: &Path,
    open_error: rustix::io::Errno,
) -> Result<NoFollowFileState> {
    use rustix::fs::{readlinkat, statat, AtFlags, FileType};

    let stat = match statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(err) if err == rustix::io::Errno::NOENT => {
            if open_error == rustix::io::Errno::NOENT {
                return Ok(NoFollowFileState::missing());
            }
            return Err(Error::Io(open_error.into()));
        }
        Err(err) => return Err(Error::Io(err.into())),
    };
    let file_type = FileType::from_raw_mode(stat.st_mode);
    let mut identity = format!(
        "unsafe-component-v1:path={};kind={file_type:?};mode={};dev={};ino={};len={};uid={};gid={};",
        hex::encode(os_str_bytes(component_path.as_os_str())),
        stat.st_mode,
        stat.st_dev,
        stat.st_ino,
        stat.st_size,
        stat.st_uid,
        stat.st_gid,
    )
    .into_bytes();
    identity.extend_from_slice(
        format!(
            "mtime={};mtime_nsec={};ctime={};ctime_nsec={};",
            stat.st_mtime, stat.st_mtime_nsec, stat.st_ctime, stat.st_ctime_nsec,
        )
        .as_bytes(),
    );
    if file_type == FileType::Symlink {
        let target = readlinkat(&directory, name, Vec::new()).map_err(|err| {
            Error::InvalidInput(format!(
                "policy dependency component `{}` changed while its symlink identity was read: {err}",
                component_path.display()
            ))
        })?;
        identity.extend_from_slice(b"target=");
        identity.extend_from_slice(hex::encode(target.as_bytes()).as_bytes());
        identity.push(b';');
    }
    Ok(NoFollowFileState::unsafe_component(identity))
}

fn unsafe_component_path_from_metadata_identity(identity: &[u8]) -> Option<PathBuf> {
    let encoded = identity.strip_prefix(b"unsafe-component-v1:path=")?;
    let encoded = encoded.split(|byte| *byte == b';').next()?;
    let bytes = hex::decode(encoded).ok()?;
    Some(PathBuf::from(os_string_from_bytes(&bytes)))
}

fn metadata_identity(metadata: Option<&fs::Metadata>, _path: &Path) -> Result<Vec<u8>> {
    let Some(metadata) = metadata else {
        return Ok(b"missing-v1".to_vec());
    };
    let kind = if metadata.file_type().is_symlink() {
        "symlink"
    } else if metadata.is_file() {
        "file"
    } else if metadata.is_dir() {
        "directory"
    } else {
        "other"
    };
    let mut identity = format!("kind={kind};len={};", metadata.len()).into_bytes();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        identity.extend_from_slice(
            format!(
                "mode={};dev={};ino={};mtime={};mtime_nsec={};ctime={};ctime_nsec={};",
                metadata.mode(),
                metadata.dev(),
                metadata.ino(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec()
            )
            .as_bytes(),
        );
    }
    Ok(identity)
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn validate_manifest_canonical(manifest: &PolicyManifest) -> Result<()> {
    let mut identities = BTreeSet::new();
    for dependency in &manifest.dependencies {
        if dependency.identity.is_empty()
            || dependency.generation != manifest.generation
            || !identities.insert((dependency.kind, dependency.identity.clone()))
        {
            return Err(Error::Corrupt(
                "duplicate or non-canonical policy dependency identity".into(),
            ));
        }
        if dependency_is_file(dependency) {
            let path = dependency_identity_path(&dependency.identity)
                .ok_or_else(|| Error::Corrupt("invalid policy path identity".into()))?;
            if !path.is_absolute()
                || lexical_normalize(&path) != path
                || dependency_path_identity(&path) != dependency.identity
            {
                return Err(Error::Corrupt("non-canonical policy path identity".into()));
            }
        }
    }
    Ok(())
}

fn policy_fingerprint(dependencies: &[PolicyDependency]) -> Result<[u8; 32]> {
    let mut ordered = dependencies.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|dependency| (dependency.kind, dependency.identity.as_str()));
    let mut hash = Sha256::new();
    hash.update(b"trail-policy-fingerprint-v2");
    hash.update((ordered.len() as u64).to_be_bytes());
    for dependency in ordered {
        for field in [
            dependency.kind.as_str().as_bytes(),
            dependency.identity.as_bytes(),
            dependency.content_identity.as_slice(),
            dependency.metadata_identity.as_slice(),
        ] {
            let len = u64::try_from(field.len())
                .map_err(|_| Error::InvalidInput("policy fingerprint field too large".into()))?;
            hash.update(len.to_be_bytes());
            hash.update(field);
        }
    }
    Ok(hash.finalize().into())
}

fn load_policy_manifest(
    conn: &Connection,
    expected: &ExpectedScope,
) -> Result<Option<PolicyManifest>> {
    let scope_id = expected.scope_id.to_text();
    let mut statement = conn.prepare(
        "SELECT dependency_identity, dependency_kind, content_identity, metadata_identity,
                observable, generation, last_source_sequence
         FROM changed_path_policy_dependencies
         WHERE scope_id=?1 AND generation=?2
         ORDER BY dependency_kind COLLATE BINARY, dependency_identity COLLATE BINARY",
    )?;
    let mut rows = statement.query(params![scope_id, expected.policy_generation as i64])?;
    let mut dependencies = Vec::new();
    while let Some(row) = rows.next()? {
        let content = row.get::<_, Vec<u8>>(2)?;
        let content_identity: [u8; 32] = content.try_into().map_err(|content: Vec<u8>| {
            Error::Corrupt(format!(
                "changed-path policy content identity has {} bytes; expected 32",
                content.len()
            ))
        })?;
        dependencies.push(PolicyDependency {
            identity: row.get(0)?,
            kind: PolicyDependencyKind::parse(&row.get::<_, String>(1)?)?,
            content_identity,
            metadata_identity: row.get(3)?,
            observable: row.get::<_, i64>(4)? != 0,
            generation: row.get::<_, i64>(5)?.try_into().map_err(|_| {
                Error::Corrupt("negative changed-path policy generation".to_string())
            })?,
            last_source_sequence: row.get::<_, i64>(6)?.try_into().map_err(|_| {
                Error::Corrupt("negative changed-path policy source sequence".to_string())
            })?,
        });
    }
    if dependencies.is_empty() {
        Ok(None)
    } else {
        Ok(Some(PolicyManifest {
            dependencies,
            generation: expected.policy_generation,
            rule_sources: Vec::new(),
        }))
    }
}

fn persist_policy_manifest_and_stale(
    conn: &Connection,
    expected: &ExpectedScope,
    manifest: &PolicyManifest,
) -> Result<()> {
    conn.execute_batch("SAVEPOINT changed_path_policy_manifest;")?;
    let result = (|| -> Result<()> {
        policy_scope_cas_guard(conn, expected)?;
        persist_policy_manifest_rows(conn, expected, manifest)?;
        mark_policy_stale_guarded(conn, expected, "policy_observer_cut_unavailable")?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("RELEASE changed_path_policy_manifest;")?;
            Ok(())
        }
        Err(err) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO changed_path_policy_manifest; RELEASE changed_path_policy_manifest;",
            );
            Err(err)
        }
    }
}

fn policy_scope_cas_guard(conn: &Connection, expected: &ExpectedScope) -> Result<()> {
    let changed = conn.execute(
        "UPDATE changed_path_scopes SET updated_at=updated_at
         WHERE scope_id=?1 AND epoch=?2 AND ref_name=?3 AND ref_generation=?4
           AND baseline_root_id=?5 AND policy_fingerprint=?6
           AND policy_dependency_generation=?7
           AND filesystem_identity=?8 AND provider_identity=?9",
        params![
            expected.scope_id.to_text(),
            expected.epoch as i64,
            expected.ref_name,
            expected.ref_generation as i64,
            expected.baseline_root.0,
            hex::encode(expected.policy_fingerprint),
            expected.policy_generation as i64,
            hex::encode(&expected.filesystem_identity),
            hex::encode(&expected.provider_identity),
        ],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(policy_stale_cas_error(expected))
    }
}

fn persist_policy_manifest_rows(
    conn: &Connection,
    expected: &ExpectedScope,
    manifest: &PolicyManifest,
) -> Result<()> {
    validate_manifest_canonical(manifest)?;
    conn.execute(
        "DELETE FROM changed_path_policy_dependencies WHERE scope_id=?1",
        [expected.scope_id.to_text()],
    )?;
    let now = crate::db::util::now_ts();
    let mut insert = conn.prepare(
        "INSERT INTO changed_path_policy_dependencies(
             scope_id, dependency_identity, dependency_kind, content_identity,
             metadata_identity, observable, generation, last_source_sequence,
             created_at, updated_at
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
    )?;
    for dependency in &manifest.dependencies {
        insert.execute(params![
            expected.scope_id.to_text(),
            dependency.identity,
            dependency.kind.as_str(),
            dependency.content_identity.as_slice(),
            dependency.metadata_identity,
            i64::from(dependency.observable),
            dependency.generation as i64,
            dependency.last_source_sequence as i64,
            now,
        ])?;
    }
    Ok(())
}

fn mark_policy_stale_guarded(
    conn: &Connection,
    expected: &ExpectedScope,
    reason: &str,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE changed_path_scopes
         SET trust_state='stale_baseline', trust_reason=?1,
             continuity_generation=continuity_generation+1, updated_at=?2
         WHERE scope_id=?3 AND epoch=?4 AND ref_name=?5 AND ref_generation=?6
           AND baseline_root_id=?7 AND policy_fingerprint=?8
           AND policy_dependency_generation=?9
           AND filesystem_identity=?10 AND provider_identity=?11",
        params![
            reason,
            crate::db::util::now_ts(),
            expected.scope_id.to_text(),
            expected.epoch as i64,
            expected.ref_name,
            expected.ref_generation as i64,
            expected.baseline_root.0,
            hex::encode(expected.policy_fingerprint),
            expected.policy_generation as i64,
            hex::encode(&expected.filesystem_identity),
            hex::encode(&expected.provider_identity),
        ],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(policy_stale_cas_error(expected))
    }
}

fn policy_stale_cas_error(expected: &ExpectedScope) -> Error {
    Error::ChangeLedgerReconcileRequired {
        scope: expected.scope_id.to_text(),
        state: "stale_baseline".to_string(),
        reason: "policy_full_expected_scope_cas_mismatch".to_string(),
        command: "trail index reconcile".to_string(),
    }
}

pub(crate) fn dependency_path_identity(path: &Path) -> String {
    format!("path:{}", hex::encode(os_str_bytes(path.as_os_str())))
}

fn dependency_path_identity_with_case(path: &Path, _case_sensitive: bool) -> String {
    dependency_path_identity(&lexical_normalize(path))
}

fn dependency_identity_path(identity: &str) -> Option<PathBuf> {
    let bytes = hex::decode(identity.strip_prefix("path:")?).ok()?;
    Some(PathBuf::from(os_string_from_bytes(&bytes)))
}

#[cfg(unix)]
fn os_str_bytes(value: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_str_bytes(value: &OsStr) -> Vec<u8> {
    value.to_string_lossy().as_bytes().to_vec()
}

#[cfg(unix)]
fn os_string_from_bytes(value: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(value.to_vec())
}

#[cfg(not(unix))]
fn os_string_from_bytes(value: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(value).into_owned())
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn normalized_path_key(path: &Path, case_sensitive: bool) -> String {
    let mut key = lexical_normalize(path)
        .to_string_lossy()
        .replace('\\', "/")
        .nfc()
        .collect::<String>();
    if !case_sensitive {
        key = key.to_lowercase();
    }
    key
}

const fn platform_default_case_sensitive() -> bool {
    !cfg!(any(target_os = "windows", target_os = "macos"))
}
