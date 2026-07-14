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

    #[test]
    fn builtins_normalization_mode_and_case_policy_are_authoritative_identities() {
        let fixture = Fixture::new();
        let policy = fixture.compile(&mut PolicyDependencyMetrics::default());
        for kind in [
            PolicyDependencyKind::Builtin,
            PolicyDependencyKind::TrailConfig,
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
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

const BUILTIN_POLICY_VERSION: &[u8] = b"trail-recording-policy-v1";
const NORMALIZATION_POLICY: &[u8] = b"relative-forward-slash-unicode-nfc-v1";
const MODE_POLICY: &[u8] = b"regular-files-only-no-follow-executable-bit-v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum PolicyDependencyKind {
    Builtin,
    TrailConfig,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordingPolicySnapshot {
    pub(crate) workspace_root: PathBuf,
    pub(crate) ignore_gitignored: bool,
    pub(crate) dependency_files: Vec<PathBuf>,
    pub(crate) case_sensitive: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdapterEquivalence {
    Equivalent,
    Conservative,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompiledPolicy {
    pub(crate) snapshot: RecordingPolicySnapshot,
    pub(crate) fingerprint: [u8; 32],
    pub(crate) dependencies: Vec<PolicyDependency>,
    pub(crate) adapter_equivalence: AdapterEquivalence,
    pub(crate) stale_baseline: bool,
    pub(crate) reused_manifest: bool,
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
        }
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
    let stored = load_policy_manifest(conn, expected)?;
    let had_stored_manifest = stored.is_some();
    if let Some(manifest) = stored {
        metrics.policy_dependency_direct_checks = metrics
            .policy_dependency_direct_checks
            .saturating_add(manifest.dependencies.len() as u64);
        match validate_policy_manifest(context, &manifest)? {
            PolicyManifestValidation::Current => {
                return finish_compiled_policy(conn, expected, context, manifest, true, false)
            }
            PolicyManifestValidation::Unobservable => {
                return finish_compiled_policy(conn, expected, context, manifest, true, true)
            }
            PolicyManifestValidation::Changed => {}
        }
    }

    metrics.policy_dependency_full_discovery =
        metrics.policy_dependency_full_discovery.saturating_add(1);
    let manifest = discover_policy_manifest(expected.policy_generation, context)?;
    persist_policy_manifest(conn, expected, &manifest)?;
    finish_compiled_policy(
        conn,
        expected,
        context,
        manifest,
        false,
        had_stored_manifest,
    )
}

fn finish_compiled_policy(
    conn: &Connection,
    expected: &ExpectedScope,
    context: &PolicyCompileContext<'_>,
    manifest: PolicyManifest,
    reused_manifest: bool,
    dependency_stale: bool,
) -> Result<CompiledPolicy> {
    let fingerprint = policy_fingerprint(&manifest.dependencies);
    let has_unobservable = manifest
        .dependencies
        .iter()
        .any(|dependency| !dependency.observable);
    let stale_baseline =
        dependency_stale || has_unobservable || fingerprint != expected.policy_fingerprint;
    if stale_baseline {
        mark_policy_stale(conn, expected)?;
    }
    let dependency_files = manifest
        .dependencies
        .iter()
        .filter(|dependency| dependency_is_file(dependency))
        .filter_map(|dependency| dependency_identity_path(&dependency.identity))
        .collect();
    Ok(CompiledPolicy {
        snapshot: RecordingPolicySnapshot {
            workspace_root: context.workspace_root.to_path_buf(),
            ignore_gitignored: context.recording.ignore_gitignored,
            dependency_files,
            case_sensitive: context.case_sensitive,
        },
        fingerprint,
        dependencies: manifest.dependencies,
        adapter_equivalence: if stale_baseline {
            AdapterEquivalence::Conservative
        } else {
            AdapterEquivalence::Equivalent
        },
        stale_baseline,
        reused_manifest,
    })
}

pub(crate) fn validate_policy_manifest(
    context: &PolicyCompileContext<'_>,
    manifest: &PolicyManifest,
) -> Result<PolicyManifestValidation> {
    let synthetic = synthetic_dependencies(manifest.generation, context)?
        .into_iter()
        .map(|dependency| ((dependency.kind, dependency.identity.clone()), dependency))
        .collect::<BTreeMap<_, _>>();
    let mut unobservable = false;
    for dependency in &manifest.dependencies {
        let current = if dependency_is_file(dependency) {
            let Some(path) = dependency_identity_path(&dependency.identity) else {
                return Ok(PolicyManifestValidation::Changed);
            };
            file_dependency(
                &path,
                dependency.kind,
                dependency.generation,
                context.workspace_root,
            )?
        } else {
            let Some(current) = synthetic.get(&(dependency.kind, dependency.identity.clone()))
            else {
                return Ok(PolicyManifestValidation::Changed);
            };
            current.clone()
        };
        if dependency.content_identity != current.content_identity
            || dependency.metadata_identity != current.metadata_identity
            || dependency.observable != current.observable
            || dependency.generation != current.generation
        {
            return Ok(PolicyManifestValidation::Changed);
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
        return Ok(PolicyManifestValidation::Changed);
    }
    Ok(if unobservable {
        PolicyManifestValidation::Unobservable
    } else {
        PolicyManifestValidation::Current
    })
}

fn dependency_is_file(dependency: &PolicyDependency) -> bool {
    dependency.identity.starts_with("path:")
}

pub(crate) fn raw_event_invalidates_policy(policy: &CompiledPolicy, path: &Path) -> bool {
    let absolute = if path.is_absolute() {
        lexical_normalize(path)
    } else {
        lexical_normalize(&policy.snapshot.workspace_root.join(path))
    };
    raw_path_may_invalidate_policy(path)
        || policy
            .dependencies
            .iter()
            .any(|dependency| dependency.identity == dependency_path_identity(&absolute))
}

pub(crate) fn raw_path_may_invalidate_policy(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let file_name = path.file_name().and_then(OsStr::to_str);
    matches!(file_name, Some(".trailignore" | ".gitignore"))
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
        context.workspace_root,
    )?);

    let root = lexical_normalize(context.workspace_root);
    for (name, kind) in [
        (".trailignore", PolicyDependencyKind::Trailignore),
        (".gitignore", PolicyDependencyKind::Gitignore),
    ] {
        dependencies.push(file_dependency(&root.join(name), kind, generation, &root)?);
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
        if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
            continue;
        }
        let kind = match entry.file_name().to_str() {
            Some(".trailignore") => PolicyDependencyKind::Trailignore,
            Some(".gitignore") => PolicyDependencyKind::Gitignore,
            _ => continue,
        };
        dependencies.push(file_dependency(entry.path(), kind, generation, &root)?);
    }

    dependencies.extend(discover_git_dependencies(generation, context)?);
    dependencies.sort_by(|left, right| {
        (left.kind, left.identity.as_str()).cmp(&(right.kind, right.identity.as_str()))
    });
    dependencies.dedup_by(|left, right| left.kind == right.kind && left.identity == right.identity);
    Ok(PolicyManifest {
        dependencies,
        generation,
    })
}

fn synthetic_dependencies(
    generation: u64,
    context: &PolicyCompileContext<'_>,
) -> Result<Vec<PolicyDependency>> {
    let recording = serde_json::to_vec(context.recording)
        .map_err(|err| Error::InvalidInput(err.to_string()))?;
    Ok(vec![
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
    ])
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
    let mut paths = BTreeMap::<PathBuf, PolicyDependencyKind>::new();
    if let Some(global) = git_environment_value(context, "GIT_CONFIG_GLOBAL") {
        paths.insert(
            lexical_normalize(&PathBuf::from(global)),
            PolicyDependencyKind::GitConfig,
        );
    }
    if let Some(system) = git_environment_value(context, "GIT_CONFIG_SYSTEM") {
        paths.insert(
            lexical_normalize(&PathBuf::from(system)),
            PolicyDependencyKind::GitConfig,
        );
    }
    let origins = run_git(
        context,
        &[
            "config",
            "--includes",
            "--show-origin",
            "--show-scope",
            "--null",
            "--list",
        ],
        true,
    )?;
    let fields = origins
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    for triple in fields.chunks_exact(3) {
        if let Some(raw_path) = triple[1].strip_prefix(b"file:") {
            let path = os_string_from_bytes(raw_path);
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                lexical_normalize(&path)
            } else {
                lexical_normalize(&context.workspace_root.join(path))
            };
            paths.insert(path, PolicyDependencyKind::GitConfig);
        }
    }

    let git_dirs = run_git(
        context,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
        true,
    )?;
    let mut lines = git_dirs.split(|byte| *byte == b'\n');
    let _git_dir = lines.next();
    if let Some(common) = lines.next().filter(|line| !line.is_empty()) {
        paths.insert(
            lexical_normalize(&PathBuf::from(os_string_from_bytes(common)).join("info/exclude")),
            PolicyDependencyKind::GitInfoExclude,
        );
    }

    let excludes = run_git(
        context,
        &[
            "config",
            "--includes",
            "--path",
            "--get",
            "core.excludesFile",
        ],
        false,
    )?;
    let excludes = trim_ascii_line_end(&excludes);
    if !excludes.is_empty() {
        let path = PathBuf::from(os_string_from_bytes(excludes));
        paths.insert(
            if path.is_absolute() {
                lexical_normalize(&path)
            } else {
                lexical_normalize(&context.workspace_root.join(path))
            },
            PolicyDependencyKind::GitExcludesFile,
        );
    }

    paths
        .into_iter()
        .map(|(path, kind)| file_dependency(&path, kind, generation, context.workspace_root))
        .collect()
}

fn git_environment_value(context: &PolicyCompileContext<'_>, key: &str) -> Option<OsString> {
    context
        .git_environment
        .iter()
        .rev()
        .find(|(candidate, _)| candidate == OsStr::new(key))
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var_os(key))
}

fn run_git(context: &PolicyCompileContext<'_>, args: &[&str], required: bool) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command.args(args).current_dir(context.workspace_root);
    for (key, value) in context.git_environment {
        command.env(key, value);
    }
    let output = command.output().map_err(Error::Io)?;
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
    workspace_root: &Path,
) -> Result<PolicyDependency> {
    let path = lexical_normalize(path);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => Some(metadata),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(Error::Io(err)),
    };
    let bytes = match &metadata {
        Some(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
            match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(err) => return Err(Error::Io(err)),
            }
        }
        _ => Vec::new(),
    };
    let observable = path.starts_with(lexical_normalize(workspace_root))
        && metadata
            .as_ref()
            .map_or(true, |metadata| !metadata.file_type().is_symlink())
        && !has_symlink_ancestor(workspace_root, &path)?;
    Ok(PolicyDependency {
        identity: dependency_path_identity(&path),
        kind,
        content_identity: digest(&bytes),
        metadata_identity: metadata_identity(metadata.as_ref(), &path)?,
        observable,
        generation,
        last_source_sequence: 0,
    })
}

fn has_symlink_ancestor(workspace_root: &Path, path: &Path) -> Result<bool> {
    let root = lexical_normalize(workspace_root);
    let Ok(relative) = path.strip_prefix(&root) else {
        return Ok(false);
    };
    let mut current = root;
    for component in relative.components() {
        current.push(component);
        if current == path {
            break;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Ok(false)
}

fn metadata_identity(metadata: Option<&fs::Metadata>, path: &Path) -> Result<Vec<u8>> {
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
    if metadata.file_type().is_symlink() {
        match fs::read_link(path) {
            Ok(target) => identity.extend_from_slice(dependency_path_identity(&target).as_bytes()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                identity.extend_from_slice(b"missing-target")
            }
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Ok(identity)
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn policy_fingerprint(dependencies: &[PolicyDependency]) -> [u8; 32] {
    let mut ordered = dependencies.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|dependency| (dependency.kind, dependency.identity.as_str()));
    let mut hash = Sha256::new();
    hash.update(b"trail-policy-fingerprint-v1\0");
    for dependency in ordered {
        hash.update(dependency.kind.as_str().as_bytes());
        hash.update([0]);
        hash.update(dependency.identity.as_bytes());
        hash.update([0]);
        hash.update(dependency.content_identity);
        hash.update([u8::from(dependency.observable)]);
    }
    hash.finalize().into()
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
        }))
    }
}

fn persist_policy_manifest(
    conn: &Connection,
    expected: &ExpectedScope,
    manifest: &PolicyManifest,
) -> Result<()> {
    conn.execute_batch("SAVEPOINT changed_path_policy_manifest;")?;
    let result = (|| -> Result<()> {
        let scope_exists = conn
            .query_row(
                "SELECT 1 FROM changed_path_scopes
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
                |_| Ok(()),
            )
            .optional()?;
        if scope_exists.is_none() {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: expected.scope_id.to_text(),
                state: "stale_baseline".to_string(),
                reason: "policy_manifest_scope_cas_mismatch".to_string(),
                command: "trail index reconcile".to_string(),
            });
        }
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

fn mark_policy_stale(conn: &Connection, expected: &ExpectedScope) -> Result<()> {
    let changed = conn.execute(
        "UPDATE changed_path_scopes
         SET trust_state='stale_baseline', trust_reason='policy_dependency_changed', updated_at=?1
         WHERE scope_id=?2 AND epoch=?3 AND ref_name=?4 AND ref_generation=?5
           AND baseline_root_id=?6 AND policy_fingerprint=?7
           AND policy_dependency_generation=?8
           AND filesystem_identity=?9 AND provider_identity=?10",
        params![
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
        Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "stale_baseline".to_string(),
            reason: "policy_stale_scope_cas_mismatch".to_string(),
            command: "trail index reconcile".to_string(),
        })
    }
}

pub(crate) fn dependency_path_identity(path: &Path) -> String {
    format!("path:{}", hex::encode(os_str_bytes(path.as_os_str())))
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

fn trim_ascii_line_end(mut value: &[u8]) -> &[u8] {
    while value
        .last()
        .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
    {
        value = &value[..value.len() - 1];
    }
    value
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
