use super::*;

impl CrabDb {
    pub(crate) fn ensure_patch_edit_allowed(
        &self,
        edit: &PatchEdit,
        allow_ignored: bool,
    ) -> Result<()> {
        match edit {
            PatchEdit::Write { path, .. }
            | PatchEdit::WriteBytes { path, .. }
            | PatchEdit::ReplaceLine { path, .. }
            | PatchEdit::Delete { path } => {
                let path = normalize_relative_path(path)?;
                self.ensure_patch_path_allowed(&path, allow_ignored)
            }
            PatchEdit::Rename { from, to } => {
                let from = normalize_relative_path(from)?;
                let to = normalize_relative_path(to)?;
                self.ensure_patch_path_allowed(&from, allow_ignored)?;
                self.ensure_patch_path_allowed(&to, allow_ignored)
            }
        }
    }

    pub(crate) fn ensure_patch_path_allowed(&self, path: &str, allow_ignored: bool) -> Result<()> {
        if is_internal_path(path) {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        if allow_ignored {
            return Ok(());
        }
        let report = self.ignore_check(path)?;
        if report.ignored {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        Ok(())
    }
}
