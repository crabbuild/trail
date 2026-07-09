use super::*;

pub(crate) fn unified_patch(
    old_path: &str,
    new_path: &str,
    old_text: &str,
    new_text: &str,
) -> String {
    let diff = TextDiff::from_lines(old_text, new_text);
    let mut out = String::new();
    out.push_str(&format!("diff --trail a/{old_path} b/{new_path}\n"));
    out.push_str(&format!("--- a/{old_path}\n"));
    out.push_str(&format!("+++ b/{new_path}\n"));
    for group in diff.grouped_ops(3) {
        for op in group {
            for change in diff.iter_changes(&op) {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                out.push_str(sign);
                out.push_str(change.value());
                if !change.value().ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
    out
}
