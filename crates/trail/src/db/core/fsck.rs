use super::*;

impl Trail {
    pub fn fsck(&self) -> Result<FsckReport> {
        let mut report = FsckReport {
            checked_refs: 0,
            checked_roots: 0,
            checked_texts: 0,
            errors: Vec::new(),
        };
        let refs = self.all_refs()?;
        for reference in refs {
            report.checked_refs += 1;
            if self.operation(&reference.change_id).is_err() {
                report.errors.push(format!(
                    "ref {} points to missing operation {}",
                    reference.name, reference.change_id.0
                ));
            }
            match self.get_object::<WorktreeRoot>(WORKTREE_ROOT_KIND, &reference.root_id) {
                Ok(root) => {
                    report.checked_roots += 1;
                    if let Err(err) = self.validate_worktree_root(&root) {
                        report
                            .errors
                            .push(format!("root {} invalid: {err}", reference.root_id.0));
                    }
                    if let Ok(files) = self.load_root_files(&reference.root_id) {
                        for entry in files.values() {
                            if let FileContentRef::Text(text_id) = &entry.content {
                                report.checked_texts += 1;
                                if let Err(err) = self.validate_text_content(text_id) {
                                    report
                                        .errors
                                        .push(format!("text {} invalid: {err}", text_id.0));
                                }
                            }
                        }
                    }
                }
                Err(err) => report.errors.push(format!(
                    "ref {} points to missing root {}: {err}",
                    reference.name, reference.root_id.0
                )),
            }
        }
        Ok(report)
    }
}
