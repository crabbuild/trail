use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewPathClass {
    #[default]
    Source,
    Dependency,
    Generated,
    Scratch,
    Secret,
    Internal,
}

impl ViewPathClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Dependency => "dependency",
            Self::Generated => "generated",
            Self::Scratch => "scratch",
            Self::Secret => "secret",
            Self::Internal => "internal",
        }
    }

    pub(crate) fn checkpoints(self) -> bool {
        self == Self::Source
    }
}

pub(crate) fn classify_view_path(path: &str) -> ViewPathClass {
    let path = path.trim_matches('/');
    if path.is_empty() {
        return ViewPathClass::Source;
    }
    let components = path.split('/').collect::<Vec<_>>();
    if matches!(components.first(), Some(&".trail" | &".git")) {
        return ViewPathClass::Internal;
    }
    let name = components.last().copied().unwrap_or(path);
    if name.starts_with("._") || name == ".DS_Store" {
        return ViewPathClass::Scratch;
    }
    if name == ".env"
        || name.starts_with(".env.")
        || matches!(name, "id_rsa" | "id_ed25519")
        || matches!(
            Path::new(name).extension().and_then(|value| value.to_str()),
            Some("pem" | "key" | "p12" | "pfx")
        )
    {
        return ViewPathClass::Secret;
    }
    if components
        .iter()
        .any(|component| matches!(*component, "node_modules" | ".pnpm"))
    {
        return ViewPathClass::Dependency;
    }
    if components.iter().any(|component| {
        matches!(
            *component,
            "target" | "dist" | "build" | "coverage" | ".next" | ".turbo"
        )
    }) {
        return ViewPathClass::Generated;
    }
    if components
        .iter()
        .any(|component| matches!(*component, "tmp" | ".tmp" | ".cache"))
    {
        return ViewPathClass::Scratch;
    }
    ViewPathClass::Source
}

#[derive(Clone, Debug)]
pub(crate) struct ViewUpperLayout {
    pub(crate) source_upper: PathBuf,
    pub(crate) generated_upper: PathBuf,
    pub(crate) scratch_upper: PathBuf,
    pub(crate) meta_dir: PathBuf,
}

impl ViewUpperLayout {
    pub(crate) fn from_source_upper(source_upper: PathBuf) -> Self {
        if source_upper.file_name().and_then(|name| name.to_str()) == Some("source-upper")
            && let Some(view_dir) = source_upper.parent()
        {
            return Self {
                source_upper: source_upper.clone(),
                generated_upper: view_dir.join("generated-upper"),
                scratch_upper: view_dir.join("scratch-upper"),
                meta_dir: view_dir.join("meta"),
            };
        }
        let meta_dir = source_upper.join(".trail");
        Self {
            generated_upper: meta_dir.join("generated-upper"),
            scratch_upper: meta_dir.join("scratch-upper"),
            meta_dir,
            source_upper,
        }
    }

    pub(crate) fn ensure(&self) -> Result<()> {
        for path in [
            &self.source_upper,
            &self.generated_upper,
            &self.scratch_upper,
            &self.meta_dir,
        ] {
            fs::create_dir_all(path)?;
        }
        Ok(())
    }

    pub(crate) fn upper_for_class(&self, class: ViewPathClass) -> &Path {
        match class {
            ViewPathClass::Source => &self.source_upper,
            ViewPathClass::Dependency | ViewPathClass::Generated => &self.generated_upper,
            ViewPathClass::Scratch | ViewPathClass::Secret => &self.scratch_upper,
            ViewPathClass::Internal => &self.meta_dir,
        }
    }

    pub(crate) fn journal_path(&self) -> PathBuf {
        self.meta_dir.join("mutation-journal.jsonl")
    }

    pub(crate) fn whiteout_journal_path(&self) -> PathBuf {
        self.meta_dir.join("whiteout-journal.jsonl")
    }

    pub(crate) fn journal_state_path(&self) -> PathBuf {
        self.meta_dir.join("journal-state.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_classification_is_specific_and_internal_rules_win() {
        assert_eq!(classify_view_path("src/lib.rs"), ViewPathClass::Source);
        assert_eq!(
            classify_view_path("frontend/node_modules/pkg/index.js"),
            ViewPathClass::Dependency
        );
        assert_eq!(
            classify_view_path("crates/core/target/debug/core"),
            ViewPathClass::Generated
        );
        assert_eq!(classify_view_path(".env.local"), ViewPathClass::Secret);
        assert_eq!(
            classify_view_path(".git/refs/heads/main"),
            ViewPathClass::Internal
        );
        assert_eq!(
            classify_view_path(".trail/views/x"),
            ViewPathClass::Internal
        );
    }
}
