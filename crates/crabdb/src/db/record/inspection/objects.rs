use super::*;

impl CrabDb {
    pub fn show(&self, selector: &str) -> Result<ShowResult> {
        if let Some(agent) = selector.strip_prefix("agent:") {
            return Ok(ShowResult::Agent {
                value: self.agent_branch(agent)?,
            });
        }
        if selector.starts_with("ch_") {
            let operation = self.operation(&ChangeId(selector.to_string()))?;
            return Ok(ShowResult::Operation {
                value: OperationShow {
                    changed_paths: summarize_file_changes(&operation.changes),
                    messages: self.messages_for_change(&operation.change_id)?,
                    operation,
                },
            });
        }
        if selector.starts_with("msg_") {
            return Ok(ShowResult::Message {
                value: self.message(selector)?,
            });
        }
        if selector.starts_with("obj_") {
            return Ok(ShowResult::Object {
                value: self.object_info(selector)?,
            });
        }
        if let Ok(agent) = self.agent_branch(selector) {
            return Ok(ShowResult::Agent { value: agent });
        }
        if let Ok(ref_record) = self.resolve_refish(selector) {
            return Ok(ShowResult::Ref { value: ref_record });
        }
        Err(Error::InvalidInput(format!("cannot show `{selector}`")))
    }

    pub fn inspect_object(&self, object_id: &str) -> Result<ObjectInspectReport> {
        let info = self.object_info(object_id)?;
        let id = ObjectId(object_id.to_string());
        let summary = match info.kind.as_str() {
            WORKTREE_ROOT_KIND => {
                let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &id)?;
                serde_json::json!({
                    "file_count": root.file_count,
                    "total_text_bytes": root.total_text_bytes,
                    "created_by": root.created_by,
                    "path_map_root": root.path_map_root,
                    "file_index_map_root": root.file_index_map_root,
                })
            }
            TEXT_CONTENT_KIND => {
                let text: TextContent = self.get_object(TEXT_CONTENT_KIND, &id)?;
                serde_json::json!({
                    "content_hash": text.content_hash,
                    "line_count": text.line_count,
                    "byte_count": text.byte_count,
                    "full_bytes_blob_id": text.full_bytes_blob_id,
                    "representation": text.representation,
                    "order_map_root": text.order_map_root,
                    "line_index_map_root": text.line_index_map_root,
                })
            }
            OPERATION_KIND => {
                let operation: Operation = self.get_object(OPERATION_KIND, &id)?;
                serde_json::json!({
                    "change_id": operation.change_id,
                    "kind": operation.kind,
                    "branch": operation.branch,
                    "actor": operation.actor,
                    "parent_count": operation.parents.len(),
                    "changed_path_count": operation.changes.len(),
                    "before_root": operation.before_root,
                    "after_root": operation.after_root,
                    "message": operation.message,
                    "created_at": operation.created_at,
                })
            }
            BLOB_KIND => {
                let blob: Blob = self.get_object(BLOB_KIND, &id)?;
                serde_json::json!({
                    "content_hash": blob.content_hash,
                    "byte_count": blob.bytes.len(),
                })
            }
            MESSAGE_KIND => {
                let message: Message = self.get_object(MESSAGE_KIND, &id)?;
                serde_json::json!({
                    "message_id": message.id,
                    "role": message.role,
                    "agent_id": message.agent_id,
                    "session_id": message.session_id,
                    "change_id": message.change_id,
                    "body_bytes": message.body.len(),
                    "created_at": message.created_at,
                })
            }
            ANCHOR_KIND => {
                let anchor: Anchor = self.get_object(ANCHOR_KIND, &id)?;
                serde_json::json!({
                    "anchor_id": anchor.id,
                    "label": anchor.label,
                    "file_id": file_id_key(&anchor.file_id),
                    "line_id": line_id_key_value(&anchor.line_id),
                    "created_path": anchor.created_path,
                    "created_line": anchor.created_line,
                    "created_change": anchor.created_change,
                    "created_at": anchor.created_at,
                })
            }
            _ => serde_json::json!({}),
        };
        Ok(ObjectInspectReport { info, summary })
    }

    pub fn inspect_root(&self, root_id: &str) -> Result<RootInspectReport> {
        let root_id = ObjectId(root_id.to_string());
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &root_id)?;
        let files = self
            .load_root_files(&root_id)?
            .into_iter()
            .map(|(path, entry)| RootFileInspect {
                path,
                file_id: file_id_key(&entry.file_id),
                kind: entry.kind,
                mode: entry.mode,
                executable: entry.executable,
                size_bytes: entry.size_bytes,
                content_hash: entry.content_hash,
                content_object: content_object_id(&entry.content).clone(),
            })
            .collect();
        Ok(RootInspectReport {
            root_id,
            root,
            files,
        })
    }

    pub fn inspect_text(&self, text_id: &str, limit: usize) -> Result<TextInspectReport> {
        let text_id = ObjectId(text_id.to_string());
        let content: TextContent = self.get_object(TEXT_CONTENT_KIND, &text_id)?;
        let loaded_lines = self.load_text_lines(&text_id)?;
        let truncated = limit > 0 && loaded_lines.len() > limit;
        let lines = loaded_lines
            .into_iter()
            .take(if limit == 0 { usize::MAX } else { limit })
            .enumerate()
            .map(|(idx, line)| TextLineInspect {
                line_number: idx as u64 + 1,
                line_id: line.line_id_key(),
                text_hash: line.text_hash,
                text: String::from_utf8_lossy(&line.text).into_owned(),
                newline: line.newline,
                introduced_by: line.introduced_by,
                last_content_change: line.last_content_change,
                last_move_change: line.last_move_change,
            })
            .collect();
        Ok(TextInspectReport {
            text_id,
            content,
            lines,
            truncated,
        })
    }
}
