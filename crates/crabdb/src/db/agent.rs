use super::*;
use crate::db::util::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const AGENT_REVIEWED_EVENT: &str = "agent_reviewed";

struct AgentReviewProgress {
    status: String,
    reviewed: Option<AgentReviewMarker>,
    changed_paths: usize,
    changed_lines: u64,
}

#[derive(Clone, Debug)]
enum AgentAskRoute {
    Inbox,
    Next,
    Summary,
    Validate,
    Brief,
    Story,
    Risk,
    Ready,
    Diagnose,
    Receipt,
    Pr,
    Changes,
    TurnDiff { file: Option<String>, patch: bool },
    Delta { file: Option<String>, patch: bool },
    New { file: Option<String>, patch: bool },
    Files,
    File { path: String, patch: bool },
    Workdir,
    Checkpoints,
    Timeline,
    Turn,
    Why(String),
    Review,
    Focus,
    View,
}

impl AgentAskRoute {
    fn intent(&self) -> &'static str {
        match self {
            AgentAskRoute::Inbox => "inbox",
            AgentAskRoute::Next => "next",
            AgentAskRoute::Summary => "summary",
            AgentAskRoute::Validate => "validate",
            AgentAskRoute::Brief => "brief",
            AgentAskRoute::Story => "story",
            AgentAskRoute::Risk => "risk",
            AgentAskRoute::Ready => "ready",
            AgentAskRoute::Diagnose => "diagnose",
            AgentAskRoute::Receipt => "receipt",
            AgentAskRoute::Pr => "pr",
            AgentAskRoute::Changes => "changes",
            AgentAskRoute::TurnDiff { .. } => "turn_diff",
            AgentAskRoute::Delta { .. } => "delta",
            AgentAskRoute::New { .. } => "new",
            AgentAskRoute::Files => "files",
            AgentAskRoute::File { .. } => "file",
            AgentAskRoute::Workdir => "workdir",
            AgentAskRoute::Checkpoints => "checkpoints",
            AgentAskRoute::Timeline => "timeline",
            AgentAskRoute::Turn => "turn",
            AgentAskRoute::Why(_) => "why",
            AgentAskRoute::Review => "review",
            AgentAskRoute::Focus => "focus",
            AgentAskRoute::View => "view",
        }
    }

    fn tool(&self) -> &'static str {
        match self {
            AgentAskRoute::Inbox => "crabdb.agent_inbox",
            AgentAskRoute::Next => "crabdb.agent_next",
            AgentAskRoute::Summary => "crabdb.agent_summary",
            AgentAskRoute::Validate => "crabdb.agent_validate",
            AgentAskRoute::Brief => "crabdb.agent_brief",
            AgentAskRoute::Story => "crabdb.agent_story",
            AgentAskRoute::Risk => "crabdb.agent_risk",
            AgentAskRoute::Ready => "crabdb.agent_ready",
            AgentAskRoute::Diagnose => "crabdb.agent_diagnose",
            AgentAskRoute::Receipt => "crabdb.agent_receipt",
            AgentAskRoute::Pr => "crabdb.agent_pr",
            AgentAskRoute::Changes => "crabdb.agent_changes",
            AgentAskRoute::TurnDiff { .. } => "crabdb.agent_diff",
            AgentAskRoute::Delta { .. } => "crabdb.agent_delta",
            AgentAskRoute::New { .. } => "crabdb.agent_new",
            AgentAskRoute::Files => "crabdb.agent_files",
            AgentAskRoute::File { .. } => "crabdb.agent_file",
            AgentAskRoute::Workdir => "crabdb.agent_workdir",
            AgentAskRoute::Checkpoints => "crabdb.agent_checkpoints",
            AgentAskRoute::Timeline => "crabdb.agent_timeline",
            AgentAskRoute::Turn => "crabdb.agent_turn",
            AgentAskRoute::Why(_) => "crabdb.agent_why",
            AgentAskRoute::Review => "crabdb.agent_review",
            AgentAskRoute::Focus => "crabdb.agent_focus",
            AgentAskRoute::View => "crabdb.agent_view",
        }
    }

    fn cli_command(&self, selector: &str) -> String {
        let selector = agent_shell_arg(selector);
        match self {
            AgentAskRoute::Inbox => "crabdb agent inbox".to_string(),
            AgentAskRoute::Next => format!("crabdb agent next {selector}"),
            AgentAskRoute::Summary => format!("crabdb agent summary {selector}"),
            AgentAskRoute::Validate => format!("crabdb agent validate {selector}"),
            AgentAskRoute::Brief => format!("crabdb agent brief {selector}"),
            AgentAskRoute::Story => format!("crabdb agent story {selector}"),
            AgentAskRoute::Risk => format!("crabdb agent risk {selector}"),
            AgentAskRoute::Ready => format!("crabdb agent can-land {selector}"),
            AgentAskRoute::Diagnose => format!("crabdb agent recover {selector}"),
            AgentAskRoute::Receipt => format!("crabdb agent receipt {selector}"),
            AgentAskRoute::Pr => format!("crabdb agent pr {selector}"),
            AgentAskRoute::Changes => format!("crabdb agent changes {selector}"),
            AgentAskRoute::TurnDiff { file, patch } => {
                let mut command = format!("crabdb agent turn-diff {selector}");
                if let Some(path) = file {
                    command.push_str(&format!(" --file {}", agent_shell_arg(path)));
                }
                if *patch {
                    command.push_str(" --patch");
                }
                command
            }
            AgentAskRoute::Delta { file, patch } => {
                let mut command = format!("crabdb agent last {selector}");
                if let Some(path) = file {
                    command.push_str(&format!(" --file {}", agent_shell_arg(path)));
                }
                if *patch {
                    command.push_str(" --patch");
                }
                command
            }
            AgentAskRoute::New { file, patch } => {
                let mut command = format!("crabdb agent what-changed {selector}");
                if let Some(path) = file {
                    command.push_str(&format!(" --file {}", agent_shell_arg(path)));
                }
                if *patch {
                    command.push_str(" --patch");
                }
                command
            }
            AgentAskRoute::Files => format!("crabdb agent changed-files {selector}"),
            AgentAskRoute::File { path, patch } => {
                let mut command =
                    format!("crabdb agent inspect {selector} {}", agent_shell_arg(path));
                if *patch {
                    command.push_str(" --patch");
                }
                command
            }
            AgentAskRoute::Workdir => format!("crabdb agent workdir {selector}"),
            AgentAskRoute::Checkpoints => format!("crabdb agent rewind-points {selector}"),
            AgentAskRoute::Timeline => format!("crabdb agent timeline {selector}"),
            AgentAskRoute::Turn => format!("crabdb agent turn {selector}"),
            AgentAskRoute::Why(path) => {
                format!("crabdb agent explain {selector} {}", agent_shell_arg(path))
            }
            AgentAskRoute::Review => format!("crabdb agent review-plan {selector}"),
            AgentAskRoute::Focus => format!("crabdb agent focus {selector}"),
            AgentAskRoute::View => format!("crabdb agent view {selector}"),
        }
    }
}

impl CrabDb {
    pub fn fresh_agent_lane_name(&self, provider: &str, name: Option<&str>) -> String {
        let label = name
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(provider);
        let component = sanitize_agent_ref_component(label);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seed = format!(
            "{}:{}:{}:{}:{}",
            provider,
            label,
            self.workspace_root.display(),
            std::process::id(),
            now
        );
        format!(
            "agent-{}-{}",
            component,
            crate::ids::short_hash(seed.as_bytes(), 6)
        )
    }

    pub fn agent_setup_report(&self, provider: &str, editor: &str) -> Result<AgentSetupReport> {
        let profile = crate::acp::acp_provider_profile(provider)?;
        let editor = match editor {
            "generic" | "vscode" | "zed" => editor,
            other => {
                return Err(Error::InvalidInput(format!(
                    "unsupported agent editor `{other}`; supported editors: generic, vscode, zed"
                )))
            }
        };
        let command = vec![
            "crabdb".to_string(),
            "--workspace".to_string(),
            self.workspace_root.to_string_lossy().to_string(),
            "agent".to_string(),
            "acp".to_string(),
            "--provider".to_string(),
            profile.agent.clone(),
        ];
        let snippet = agent_editor_snippet(editor, &profile.agent, &command);
        Ok(AgentSetupReport {
            provider: profile.agent,
            editor: editor.to_string(),
            command,
            snippet,
            detected: profile.available,
            warnings: if profile.available {
                Vec::new()
            } else {
                profile.notes
            },
        })
    }

    pub fn list_agent_tasks(&self) -> Result<AgentTaskListReport> {
        let mut lanes = self.list_lanes()?;
        lanes.sort_by(|left, right| {
            right
                .branch
                .updated_at
                .cmp(&left.branch.updated_at)
                .then_with(|| right.record.created_at.cmp(&left.record.created_at))
        });
        let mut tasks = Vec::new();
        for lane in lanes {
            if !self.lane_looks_like_agent_task(&lane)? {
                continue;
            }
            tasks.push(self.agent_task_for_lane_details(lane, 10)?);
        }
        Ok(AgentTaskListReport { tasks })
    }

    pub fn agent_inbox(&self) -> Result<AgentInboxReport> {
        let tasks = self.list_agent_tasks()?.tasks;
        if tasks.is_empty() {
            let next = StatusSuggestion {
                command: "crabdb agent setup --provider claude-code --editor vscode".to_string(),
                reason: "configure an editor once, then start an agent task".to_string(),
            };
            return Ok(AgentInboxReport {
                total: 0,
                attention_count: 0,
                groups: Vec::new(),
                items: Vec::new(),
                tasks,
                suggestions: vec![
                    next.clone(),
                    StatusSuggestion {
                        command: "crabdb agent doctor --provider claude-code".to_string(),
                        reason: "verify the provider and workspace before connecting an editor"
                            .to_string(),
                    },
                    StatusSuggestion {
                        command: "crabdb agent start --provider claude-code".to_string(),
                        reason: "start a terminal agent task instead of using an editor"
                            .to_string(),
                    },
                ],
                next,
            });
        }

        let mut items = Vec::new();
        for task in &tasks {
            items.push(self.agent_inbox_item_for_task(task)?);
        }
        let mut groups = agent_inbox_groups(&items);
        for group in &mut groups {
            if let Some(item) = group.items.first() {
                group.next = Some(item.next.clone());
            }
        }
        let attention_count = groups
            .iter()
            .filter(|group| !matches!(group.status, AgentTaskStatus::Applied))
            .map(|group| group.items.len())
            .sum();
        let next = groups
            .iter()
            .find(|group| !matches!(group.status, AgentTaskStatus::Applied))
            .and_then(|group| group.next.clone())
            .or_else(|| groups.iter().find_map(|group| group.next.clone()))
            .unwrap_or_else(|| StatusSuggestion {
                command: "crabdb agent setup --provider claude-code --editor vscode".to_string(),
                reason: "configure an editor once, then start an agent task".to_string(),
            });
        let mut suggestions = vec![next.clone()];
        if tasks.len() > 1 {
            agent_push_suggestion(
                &mut suggestions,
                "crabdb agent inbox".to_string(),
                "return to the grouped task overview",
            );
        }
        if let Some(latest) = tasks.first() {
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent brief {}", latest.lane),
                "open one compact review packet for the newest task",
            );
        }
        Ok(AgentInboxReport {
            total: tasks.len(),
            attention_count,
            groups,
            items,
            tasks,
            next,
            suggestions,
        })
    }

    pub fn agent_status(&self) -> Result<AgentStatusReport> {
        let Some(lane) = self.resolve_agent_selector("latest")? else {
            return Ok(AgentStatusReport {
                status: AgentTaskStatus::Empty,
                latest: None,
                risk: None,
                suggestions: vec![StatusSuggestion {
                    command: "crabdb agent setup --provider claude-code --editor vscode"
                        .to_string(),
                    reason: "configure an editor once, then start an agent task".to_string(),
                }],
            });
        };
        let view = self.agent_task_view(&lane)?;
        let risk = agent_risk_report_from_view(&view);
        let next = self.agent_next_report_for_view(&view)?;
        let mut suggestions = vec![next.primary];
        suggestions.extend(next.suggestions);
        Ok(AgentStatusReport {
            status: view.task.status.clone(),
            suggestions,
            risk: Some(risk),
            latest: Some(view.task),
        })
    }

    pub fn agent_next(&self, selector: &str) -> Result<AgentNextReport> {
        let Some(lane) = self.resolve_agent_selector(selector)? else {
            let primary = StatusSuggestion {
                command: "crabdb agent setup --provider claude-code --editor vscode".to_string(),
                reason:
                    "configure an editor once; CrabDB will create fresh task lanes automatically"
                        .to_string(),
            };
            return Ok(AgentNextReport {
                status: AgentTaskStatus::Empty,
                task: None,
                focus: "setup".to_string(),
                summary: "No agent tasks have been recorded yet.".to_string(),
                primary,
                suggestions: vec![
                    StatusSuggestion {
                        command: "crabdb agent doctor --provider claude-code".to_string(),
                        reason: "verify the provider and workspace before connecting an editor"
                            .to_string(),
                    },
                    StatusSuggestion {
                        command: "crabdb agent start --provider claude-code".to_string(),
                        reason: "start a terminal agent task instead of using an editor"
                            .to_string(),
                    },
                ],
            });
        };
        let view = self.agent_task_view(&lane)?;
        self.agent_next_report_for_view(&view)
    }

    pub fn agent_ask(&mut self, selector: &str, question: &str) -> Result<AgentAskReport> {
        let route = agent_ask_route(question)?;
        let report = match &route {
            AgentAskRoute::Inbox => serde_json::to_value(self.agent_inbox()?)?,
            AgentAskRoute::Next => serde_json::to_value(self.agent_next(selector)?)?,
            AgentAskRoute::Summary => serde_json::to_value(self.agent_summary(selector)?)?,
            AgentAskRoute::Validate => serde_json::to_value(self.agent_validate(selector)?)?,
            AgentAskRoute::Brief => serde_json::to_value(self.agent_brief(selector)?)?,
            AgentAskRoute::Story => serde_json::to_value(self.agent_story(selector)?)?,
            AgentAskRoute::Risk => serde_json::to_value(self.agent_risk(selector)?)?,
            AgentAskRoute::Ready => serde_json::to_value(self.agent_ready(selector)?)?,
            AgentAskRoute::Diagnose => serde_json::to_value(self.agent_diagnose(selector)?)?,
            AgentAskRoute::Receipt => serde_json::to_value(self.agent_receipt(selector)?)?,
            AgentAskRoute::Pr => serde_json::to_value(self.agent_pr_draft(selector)?)?,
            AgentAskRoute::Changes => serde_json::to_value(self.agent_changes(selector, false)?)?,
            AgentAskRoute::TurnDiff { file, patch } => serde_json::to_value(self.agent_diff(
                selector,
                None,
                None,
                None,
                true,
                file.as_deref(),
                *patch,
            )?)?,
            AgentAskRoute::Delta { file, patch } => {
                serde_json::to_value(self.agent_delta(selector, false, file.as_deref(), *patch)?)?
            }
            AgentAskRoute::New { file, patch } => {
                serde_json::to_value(self.agent_new(selector, file.as_deref(), *patch)?)?
            }
            AgentAskRoute::Files => serde_json::to_value(self.agent_files(selector)?)?,
            AgentAskRoute::File { path, patch } => {
                serde_json::to_value(self.agent_file(selector, path, *patch)?)?
            }
            AgentAskRoute::Workdir => serde_json::to_value(self.agent_workdir(selector)?)?,
            AgentAskRoute::Checkpoints => serde_json::to_value(self.agent_checkpoints(selector)?)?,
            AgentAskRoute::Timeline => serde_json::to_value(self.agent_timeline(selector, false)?)?,
            AgentAskRoute::Turn => {
                serde_json::to_value(self.agent_turn(selector, "last", None, false)?)?
            }
            AgentAskRoute::Why(path) => serde_json::to_value(self.agent_why(selector, path)?)?,
            AgentAskRoute::Review => serde_json::to_value(self.agent_review(selector)?)?,
            AgentAskRoute::Focus => serde_json::to_value(self.agent_focus(selector, None, false)?)?,
            AgentAskRoute::View => serde_json::to_value(self.agent_task_view(selector)?)?,
        };
        let routed_command = route.cli_command(selector);
        let suggestions = vec![
            StatusSuggestion {
                command: routed_command.clone(),
                reason: "run the routed CrabDB view from the CLI".to_string(),
            },
            StatusSuggestion {
                command: format!(
                    "crabdb agent ask --selector {} {}",
                    agent_shell_arg(selector),
                    agent_shell_arg(question)
                ),
                reason: "ask the same plain-language question again".to_string(),
            },
        ];
        Ok(AgentAskReport {
            selector: selector.to_string(),
            question: question.to_string(),
            intent: route.intent().to_string(),
            tool: route.tool().to_string(),
            read_only: true,
            routed_command,
            report,
            suggestions,
        })
    }

    pub fn agent_brief(&self, selector: &str) -> Result<AgentBriefReport> {
        let view = self.agent_task_view(selector)?;
        let next = self.agent_next_report_for_view(&view)?;
        let mut groups = self.agent_turn_change_groups(&view)?;
        if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
        }
        let latest_change_diff = self.agent_latest_group_diff(&groups)?;
        let tool_summaries = agent_brief_tool_summaries(&view, &groups);
        let suggestions = agent_brief_suggestions(&view.task.lane, &next);
        let risk = agent_risk_report_from_view(&view);
        Ok(AgentBriefReport {
            task: view.task,
            next,
            risk,
            ready_to_apply: view.review.readiness.ready,
            readiness_status: view.review.readiness.status,
            blockers: view.review.readiness.blockers,
            warnings: view.review.readiness.warnings,
            changed_paths: view.review.changed_paths,
            groups,
            latest_change_diff,
            tool_summaries,
            suggestions,
        })
    }

    pub fn agent_task_view(&self, selector: &str) -> Result<AgentTaskViewReport> {
        let lane = self
            .resolve_agent_selector(selector)?
            .ok_or_else(|| Error::InvalidInput("no agent tasks have been recorded".to_string()))?;
        let review = self.lane_review_packet(&lane, 50)?;
        let transcript = self.transcript(&lane).ok();
        let task = self.agent_task_from_review(&review, transcript.as_ref())?;
        Ok(AgentTaskViewReport {
            task,
            review,
            transcript,
        })
    }

    pub fn agent_workdir(&self, selector: &str) -> Result<AgentWorkdirReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let workdir = view.task.workdir.clone();
        let cd_command = workdir
            .as_ref()
            .map(|path| format!("cd {}", shell_quote(path)));
        let mut suggestions = Vec::new();
        if let Some(command) = &cd_command {
            suggestions.push(StatusSuggestion {
                command: command.clone(),
                reason: "open the materialized agent task workdir".to_string(),
            });
        }
        suggestions.push(StatusSuggestion {
            command: format!("crabdb agent brief {lane}"),
            reason: "review the task summary, changed files, tools, and next action".to_string(),
        });
        Ok(AgentWorkdirReport {
            task: view.task,
            workdir,
            cd_command,
            suggestions,
        })
    }

    pub fn agent_story(&self, selector: &str) -> Result<AgentStoryReport> {
        let view = self.agent_task_view(selector)?;
        let next = self.agent_next_report_for_view(&view)?;
        let mut groups = self.agent_turn_change_groups(&view)?;
        if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
        }
        let tool_summaries = agent_brief_tool_summaries(&view, &groups);
        let summary = agent_story_summary(&view, &groups);
        let turn_summaries = groups
            .iter()
            .map(|group| AgentStoryTurn {
                index: group.index,
                id: group.id.clone(),
                turn_id: group.turn_id.clone(),
                prompt_preview: group.prompt_preview.clone(),
                outcome_preview: group.assistant_preview.clone(),
                checkpoint: group.checkpoint.clone(),
                changed_paths: group.changed_paths.clone(),
                tool_summaries: group.tool_summaries.clone(),
            })
            .collect::<Vec<_>>();
        let mut suggestions = vec![next.primary.clone()];
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent changes {}", view.task.lane),
            "see the prompt-to-checkpoint change breakdown",
        );
        if !turn_summaries.is_empty() {
            agent_push_suggestion(
                &mut suggestions,
                agent_turn_diff_command(&view.task.lane, None, None, true),
                "inspect the most recent turn patch",
            );
        }
        if view.task.workdir.is_some() {
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent workdir {}", view.task.lane),
                "jump into the materialized task workdir",
            );
        }
        Ok(AgentStoryReport {
            changed_files: view.task.changed_paths.clone(),
            risk_notes: agent_story_risk_notes(&view),
            task: view.task,
            summary,
            turn_summaries,
            tool_summaries,
            next: next.primary,
            suggestions,
        })
    }

    pub fn agent_risk(&self, selector: &str) -> Result<AgentRiskReport> {
        let view = self.agent_task_view(selector)?;
        Ok(agent_risk_report_from_view(&view))
    }

    pub fn agent_ready(&mut self, selector: &str) -> Result<AgentReadyReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let crab_branch = self.config.workspace.default_branch.clone();
        let risk = agent_risk_report_from_view(&view);
        let apply_result = self.agent_apply(&lane, true, None);
        let (apply_preview, apply_error) = match apply_result {
            Ok(report) => (Some(report), None),
            Err(err) => (None, Some(err.to_string())),
        };
        let ready = view.review.readiness.ready
            && apply_preview
                .as_ref()
                .is_some_and(|report| report.status == "ready");
        let status = agent_ready_status(&view, apply_preview.as_ref(), apply_error.as_deref());
        let next = agent_ready_next(
            &lane,
            &crab_branch,
            &status,
            apply_preview.as_ref(),
            apply_error.as_deref(),
        );
        let suggestions = agent_ready_suggestions(&view, &risk, &next, apply_preview.as_ref());
        let summary = agent_ready_summary(&view, &risk, ready, &status, apply_error.as_deref());
        Ok(AgentReadyReport {
            task: view.task.clone(),
            ready,
            status,
            summary,
            readiness_status: view.review.readiness.status.clone(),
            risk,
            blockers: view.review.readiness.blockers.clone(),
            warnings: view.review.readiness.warnings.clone(),
            apply_preview,
            apply_error,
            next,
            suggestions,
        })
    }

    pub fn agent_report(&self, selector: &str) -> Result<AgentReviewBundleReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let story = self.agent_story(&lane)?;
        let risk = agent_risk_report_from_view(&view);
        let changes = self.agent_changes(&lane, false)?;
        let next = self.agent_next_report_for_view(&view)?;
        let summary = agent_report_summary(&view, &risk, &changes);
        let suggestions = agent_report_suggestions(&view, &next.primary);
        let markdown = agent_report_markdown(&view, &story, &risk, &changes, &next.primary);
        Ok(AgentReviewBundleReport {
            task: view.task,
            summary,
            readiness_status: view.review.readiness.status.clone(),
            ready_to_apply: view.review.readiness.ready,
            story,
            risk,
            changes,
            review: view.review,
            transcript: view.transcript,
            markdown,
            next: next.primary,
            suggestions,
        })
    }

    pub fn agent_receipt(&self, selector: &str) -> Result<AgentReceiptReport> {
        let report = self.agent_report(selector)?;
        let validation = agent_receipt_validation(&report.review);
        let markdown = agent_receipt_markdown(&report, &validation);
        let mut suggestions = vec![report.next.clone()];
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent ready {}", report.task.lane),
            "check whether this task can be safely applied to Git",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent changes {}", report.task.lane),
            "inspect high-level change cards behind this receipt",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent pr {}", report.task.lane),
            "draft a pull request title and body from this task",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent review-plan {}", report.task.lane),
            "open the full review dashboard if anything looks risky",
        );
        Ok(AgentReceiptReport {
            task: report.task.clone(),
            summary: report.summary.clone(),
            status: report.task.status.clone(),
            readiness_status: report.readiness_status.clone(),
            ready_to_apply: report.ready_to_apply,
            risk: report.risk.clone(),
            changed_paths: report.task.changed_paths.clone(),
            turns: report.story.turn_summaries.clone(),
            tool_summaries: report.story.tool_summaries.clone(),
            validation,
            latest_checkpoint: report.task.latest_checkpoint.clone(),
            next: report.next.clone(),
            suggestions,
            markdown,
        })
    }

    pub fn agent_pr_draft(&self, selector: &str) -> Result<AgentPrDraftReport> {
        let receipt = self.agent_receipt(selector)?;
        let title = agent_pr_title(&receipt);
        let body = agent_pr_body(&receipt);
        let mut suggestions = Vec::new();
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent ready {}", receipt.task.lane),
            "confirm apply readiness before opening or updating a PR",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent land {} --dry-run", receipt.task.lane),
            "preview the Git commit and fast-forward plan",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent receipt {}", receipt.task.lane),
            "print the underlying task receipt",
        );
        Ok(AgentPrDraftReport {
            task: receipt.task.clone(),
            title,
            body,
            ready_to_apply: receipt.ready_to_apply,
            readiness_status: receipt.readiness_status.clone(),
            risk: receipt.risk.clone(),
            changed_paths: receipt.changed_paths.clone(),
            validation: receipt.validation.clone(),
            latest_checkpoint: receipt.latest_checkpoint.clone(),
            suggestions,
        })
    }

    pub fn agent_summary(&mut self, selector: &str) -> Result<AgentSummaryReport> {
        let receipt = self.agent_receipt(selector)?;
        let lane = receipt.task.lane.clone();
        let ready = self.agent_ready(&lane)?;
        let pr_title = agent_pr_title(&receipt);
        let pr_body = agent_pr_body(&receipt);
        let next = ready.next.clone();
        let suggestions = agent_summary_suggestions(&lane, &next, ready.ready);
        let summary = agent_summary_text(&receipt, &ready);
        Ok(AgentSummaryReport {
            task: receipt.task.clone(),
            summary,
            ready: ready.ready,
            ready_status: ready.status.clone(),
            readiness_status: ready.readiness_status.clone(),
            risk: ready.risk.clone(),
            blockers: ready.blockers.clone(),
            warnings: ready.warnings.clone(),
            changed_paths: receipt.changed_paths.clone(),
            validation: receipt.validation.clone(),
            latest_checkpoint: receipt.latest_checkpoint.clone(),
            apply_preview: ready.apply_preview.clone(),
            apply_error: ready.apply_error.clone(),
            receipt_markdown: receipt.markdown.clone(),
            pr_title,
            pr_body,
            next,
            suggestions,
        })
    }

    pub fn agent_validate(&self, selector: &str) -> Result<AgentValidationReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let review = &view.review;
        let needs_test = agent_validation_needs_gate(&review.latest_test, "missing_latest_test");
        let needs_eval = agent_validation_needs_gate(&review.latest_eval, "missing_latest_eval");
        let status = agent_validation_status(
            &review.latest_test,
            &review.latest_eval,
            needs_test,
            needs_eval,
        );
        let suggestions = agent_validation_suggestions(
            &lane,
            self.workspace_root(),
            &view.task.changed_paths,
            needs_test,
            needs_eval,
        );
        let next = suggestions
            .first()
            .cloned()
            .unwrap_or_else(|| StatusSuggestion {
                command: format!("crabdb agent ready {lane}"),
                reason: "check apply readiness after reviewing validation".to_string(),
            });
        let summary = agent_validation_summary(&view.task, &status, needs_test, needs_eval);
        Ok(AgentValidationReport {
            task: view.task.clone(),
            status,
            summary,
            needs_test,
            needs_eval,
            latest_test: review.latest_test.clone(),
            latest_eval: review.latest_eval.clone(),
            recent_gates: review.recent_gates.clone(),
            changed_paths: view.task.changed_paths,
            next,
            suggestions,
        })
    }

    pub fn agent_diagnose(&mut self, selector: &str) -> Result<AgentDiagnosisReport> {
        let ready = self.agent_ready(selector)?;
        let lane = ready.task.lane.clone();
        let checkpoints = self.agent_checkpoints(&lane)?;
        let (status, severity, likely_issue, evidence) =
            agent_diagnosis_assessment(&ready, &checkpoints);
        let recovery_options = agent_diagnosis_recovery_options(&lane, &ready, &checkpoints);
        let next = if ready.ready && status == "ok" {
            ready.next.clone()
        } else {
            recovery_options
                .first()
                .cloned()
                .unwrap_or_else(|| ready.next.clone())
        };
        let suggestions = agent_diagnosis_suggestions(&lane, &next, &recovery_options);
        let summary = agent_diagnosis_summary(&ready, &status, &likely_issue);
        Ok(AgentDiagnosisReport {
            task: ready.task.clone(),
            status,
            severity,
            summary,
            likely_issue,
            evidence,
            ready: ready.ready,
            ready_status: ready.status.clone(),
            readiness_status: ready.readiness_status.clone(),
            risk: ready.risk.clone(),
            blockers: ready.blockers.clone(),
            warnings: ready.warnings.clone(),
            checkpoints: checkpoints.entries.into_iter().rev().take(5).collect(),
            recovery_options,
            next,
            suggestions,
        })
    }

    pub fn agent_review(&self, selector: &str) -> Result<AgentReviewReport> {
        let view = self.agent_task_view(selector)?;
        let risk = agent_risk_report_from_view(&view);
        let mut groups = self.agent_turn_change_groups(&view)?;
        if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
        }
        let priorities =
            agent_review_priorities(&view.task.lane, &view.task.changed_paths, &groups);
        let next = agent_review_next(&view, &risk, &priorities);
        let suggestions = agent_review_suggestions(&view, &next, &priorities);
        let transcript_turns = view
            .transcript
            .as_ref()
            .map(|transcript| transcript.turns.len())
            .unwrap_or(0);
        let summary = agent_review_summary(&view, &risk, &priorities);
        Ok(AgentReviewReport {
            task: view.task.clone(),
            summary,
            readiness_status: view.review.readiness.status.clone(),
            ready_to_apply: view.review.readiness.ready,
            risk,
            transcript_turns,
            tool_events: view.task.tool_events,
            latest_checkpoint: view.task.latest_checkpoint.clone(),
            priorities,
            blockers: view.review.readiness.blockers.clone(),
            warnings: view.review.readiness.warnings.clone(),
            next,
            suggestions,
        })
    }

    pub fn agent_focus(
        &self,
        selector: &str,
        file: Option<&str>,
        patches: bool,
    ) -> Result<AgentFocusReport> {
        let review = self.agent_review(selector)?;
        let lane = review.task.lane.clone();
        let (path, source, priority) = if let Some(file) = file {
            let why = self.agent_why(&lane, file)?;
            let priority = review
                .priorities
                .iter()
                .find(|priority| agent_file_matches_path(&priority.change, &why.path))
                .cloned();
            (why.path, "file".to_string(), priority)
        } else {
            let priority = review.priorities.first().cloned().ok_or_else(|| {
                Error::InvalidInput(format!(
                    "agent task `{}` has no changed files to focus",
                    agent_task_label(&review.task)
                ))
            })?;
            (
                priority.change.path.clone(),
                "review_priority".to_string(),
                Some(priority),
            )
        };
        let why = self.agent_why(&lane, &path)?;
        if !why.matched {
            return Err(Error::InvalidInput(format!(
                "`{}` is not recorded as changed in agent task `{}`",
                path,
                agent_task_label(&review.task)
            )));
        }
        let (turn, operation) = agent_focus_diff_target(&why.groups);
        let diff = self.agent_diff(
            &lane,
            turn.as_deref(),
            operation.as_deref(),
            None,
            false,
            Some(&path),
            patches,
        )?;
        let next = if patches {
            review.next.clone()
        } else {
            StatusSuggestion {
                command: format!(
                    "crabdb agent focus {lane} --file {} --patch",
                    agent_shell_arg(&path)
                ),
                reason: "show the focused patch for this file".to_string(),
            }
        };
        let suggestions = agent_focus_suggestions(&lane, &path, &next, &review);
        let summary = agent_focus_summary(&review, &path, &source, priority.as_ref(), &why);
        Ok(AgentFocusReport {
            task: review.task,
            path,
            source,
            summary,
            priority,
            why,
            diff,
            next,
            suggestions,
        })
    }

    pub fn agent_checkpoints(&self, selector: &str) -> Result<AgentCheckpointReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let branch = self.lane_branch(&lane)?;
        let mut groups = self.agent_turn_change_groups(&view)?;
        if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
        }
        let entries = groups
            .iter()
            .map(|group| agent_checkpoint_entry(&lane, group))
            .collect::<Vec<_>>();
        let mut suggestions = Vec::new();
        if !entries.is_empty() {
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent rewind {lane} --to before-last-turn"),
                "rewind to the state before the most recent completed turn",
            );
        }
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent changes {lane}"),
            "review prompt-to-checkpoint changes before rewinding",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent report {lane} --markdown"),
            "create a copyable review report before recovery",
        );
        Ok(AgentCheckpointReport {
            task: view.task,
            base_change: branch.base_change,
            head_change: branch.head_change,
            entries,
            suggestions,
        })
    }

    pub fn agent_files(&self, selector: &str) -> Result<AgentFilesReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let mut groups = self.agent_turn_change_groups(&view)?;
        let grouping = if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
            "operation"
        } else {
            "turn"
        };
        let files = view
            .task
            .changed_paths
            .iter()
            .map(|change| agent_file_entry(&lane, change, &groups))
            .collect::<Vec<_>>();
        let mut suggestions = Vec::new();
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent changes {lane}"),
            "review the same work grouped by prompt or operation",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent report {lane} --markdown"),
            "create a copyable review report for this task",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent land {lane} --dry-run"),
            "preview applying the task safely",
        );
        Ok(AgentFilesReport {
            task: view.task,
            lane,
            grouping: grouping.to_string(),
            files,
            suggestions,
        })
    }

    pub fn agent_file(&self, selector: &str, path: &str, patches: bool) -> Result<AgentFileReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let path = self.normalize_agent_query_path(&view, path)?;
        let mut groups = self.agent_turn_change_groups(&view)?;
        if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
        }
        let change = view
            .task
            .changed_paths
            .iter()
            .find(|change| agent_file_matches_path(change, &path))
            .cloned();
        let matched_groups = filter_agent_groups_by_path(groups.clone(), &path);
        let file = change
            .as_ref()
            .map(|change| agent_file_entry(&lane, change, &groups));
        let change_cards = agent_change_cards(&lane, &view.task.changed_paths, &groups)
            .into_iter()
            .filter(|card| {
                card.changed_paths
                    .iter()
                    .any(|change| agent_file_matches_path(change, &path))
            })
            .collect::<Vec<_>>();
        let why = self.agent_why(&lane, &path)?;
        let diff = if patches && change.is_some() {
            if let Some(file) = &file {
                Some(self.agent_change_set_diff_for_file(&lane, file)?)
            } else {
                Some(self.agent_diff(&lane, None, None, None, false, Some(&path), true)?)
            }
        } else {
            None
        };
        let matched = change.is_some() || why.matched || !matched_groups.is_empty();
        let next = agent_file_next(&lane, &path, matched, patches, change_cards.first());
        let suggestions =
            agent_file_suggestions(&lane, &path, matched, patches, &change_cards, &next);
        let summary = agent_file_summary(
            &path,
            matched,
            change.as_ref(),
            &matched_groups,
            &change_cards,
            patches,
        );
        Ok(AgentFileReport {
            task: view.task,
            lane,
            path,
            matched,
            summary,
            change,
            file,
            change_cards,
            groups: matched_groups,
            why,
            diff,
            next,
            suggestions,
        })
    }

    pub fn agent_compare(
        &self,
        left_selector: &str,
        right_selector: &str,
    ) -> Result<AgentCompareReport> {
        let left_view = self.agent_task_view(left_selector)?;
        let right_view = self.agent_task_view(right_selector)?;
        if left_view.task.lane == right_view.task.lane {
            return Err(Error::InvalidInput(
                "agent compare requires two different agent tasks".to_string(),
            ));
        }

        let left_risk = agent_risk_report_from_view(&left_view);
        let right_risk = agent_risk_report_from_view(&right_view);
        let left_paths = agent_compare_path_map(&left_view.task.changed_paths);
        let right_paths = agent_compare_path_map(&right_view.task.changed_paths);

        let mut shared_paths = Vec::new();
        let mut left_only_paths = Vec::new();
        let mut right_only_paths = Vec::new();

        for (path, left) in &left_paths {
            if let Some(right) = right_paths.get(path) {
                shared_paths.push(AgentComparePath {
                    path: path.clone(),
                    left: left.clone(),
                    right: right.clone(),
                    note: "both tasks changed this path".to_string(),
                });
            } else {
                left_only_paths.push(left.clone());
            }
        }
        for (path, right) in &right_paths {
            if !left_paths.contains_key(path) {
                right_only_paths.push(right.clone());
            }
        }

        let summary = agent_compare_summary(
            &left_view.task,
            &right_view.task,
            &left_risk,
            &right_risk,
            shared_paths.len(),
            left_only_paths.len(),
            right_only_paths.len(),
        );
        let recommendation = agent_compare_recommendation(
            &left_view.task,
            &right_view.task,
            &left_risk,
            &right_risk,
            &shared_paths,
        );
        let mut suggestions = vec![recommendation.clone()];
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent review-plan {}", left_view.task.lane),
            "inspect the left task readiness, transcript, and changed paths",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent review-plan {}", right_view.task.lane),
            "inspect the right task readiness, transcript, and changed paths",
        );
        if let Some(shared) = shared_paths.first() {
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent why {} {}", left_view.task.lane, shared.path),
                "explain the first shared file in the left task",
            );
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent why {} {}", right_view.task.lane, shared.path),
                "explain the first shared file in the right task",
            );
        } else {
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent land {} --dry-run", left_view.task.lane),
                "preview applying the left task",
            );
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent land {} --dry-run", right_view.task.lane),
                "preview applying the right task",
            );
        }

        Ok(AgentCompareReport {
            left: left_view.task,
            right: right_view.task,
            left_risk,
            right_risk,
            summary,
            shared_paths,
            left_only_paths,
            right_only_paths,
            recommendation,
            suggestions,
        })
    }

    pub fn run_agent_test_with_options(
        &mut self,
        selector: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: LaneGateOptions,
    ) -> Result<LaneTestReport> {
        let lane = self
            .resolve_agent_selector(selector)?
            .ok_or_else(|| Error::InvalidInput("no agent tasks have been recorded".to_string()))?;
        self.run_lane_test_with_options(&lane, command, turn_id, timeout_secs, options)
    }

    pub fn run_agent_eval_with_options(
        &mut self,
        selector: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: LaneGateOptions,
    ) -> Result<LaneTestReport> {
        let lane = self
            .resolve_agent_selector(selector)?
            .ok_or_else(|| Error::InvalidInput("no agent tasks have been recorded".to_string()))?;
        self.run_lane_eval_with_options(&lane, command, turn_id, timeout_secs, options)
    }

    pub fn agent_changes(&self, selector: &str, by_operation: bool) -> Result<AgentChangesReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let branch = self.lane_branch(&lane)?;
        let mut groups = if by_operation {
            self.agent_operation_change_groups(&view)?
        } else {
            self.agent_turn_change_groups(&view)?
        };
        let grouping = if by_operation {
            "operation"
        } else if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
            "operation"
        } else {
            "turn"
        };
        let cards = agent_change_cards(&lane, &view.task.changed_paths, &groups);
        let summary = agent_changes_summary(&view, grouping, &cards, &groups);
        let next = agent_changes_next(&lane, &cards);
        let suggestions = agent_changes_suggestions(&lane, &cards, &next);
        Ok(AgentChangesReport {
            lane: lane.clone(),
            summary,
            next,
            base_change: branch.base_change,
            head_change: branch.head_change,
            total_changed_paths: view.task.changed_paths.clone(),
            cards,
            suggestions,
            task: view.task,
            grouping: grouping.to_string(),
            groups,
        })
    }

    pub fn agent_delta(
        &self,
        selector: &str,
        by_operation: bool,
        file: Option<&str>,
        patches: bool,
    ) -> Result<AgentDeltaReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let mut groups = if by_operation {
            self.agent_operation_change_groups(&view)?
        } else {
            self.agent_turn_change_groups(&view)?
        };
        let mode = if by_operation {
            "operation"
        } else if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
            "operation"
        } else {
            "turn"
        };
        let file_filter = file
            .map(|path| self.normalize_agent_query_path(&view, path))
            .transpose()?;
        let group = groups
            .iter()
            .rev()
            .find(|group| group.before_change.is_some() && group.after_change.is_some())
            .cloned();
        let diff = if let Some(group) = &group {
            match group.kind.as_str() {
                "turn" => Some(self.agent_diff(
                    &lane,
                    Some(&group.index.to_string()),
                    None,
                    None,
                    false,
                    file_filter.as_deref(),
                    patches,
                )?),
                _ => {
                    if let Some(operation) = &group.operation_id {
                        Some(self.agent_diff(
                            &lane,
                            None,
                            Some(&operation.0),
                            None,
                            false,
                            file_filter.as_deref(),
                            patches,
                        )?)
                    } else {
                        None
                    }
                }
            }
        } else {
            None
        };
        let changed_paths = diff
            .as_ref()
            .map(|diff| diff.diff.files.clone())
            .unwrap_or_default();
        let matched = file_filter.is_none() || !changed_paths.is_empty();
        let fallback_next = self.agent_next_report_for_view(&view)?.primary;
        let next = agent_delta_next(
            &lane,
            mode,
            group.as_ref(),
            file_filter.as_deref(),
            matched,
            patches,
            &fallback_next,
        );
        let suggestions = agent_delta_suggestions(
            &lane,
            mode,
            group.as_ref(),
            file_filter.as_deref(),
            matched,
            patches,
            &changed_paths,
            &next,
        );
        let summary = agent_delta_summary(
            &view.task,
            mode,
            group.as_ref(),
            file_filter.as_deref(),
            matched,
            &changed_paths,
            patches,
        );
        Ok(AgentDeltaReport {
            task: view.task,
            lane,
            mode: mode.to_string(),
            summary,
            group,
            file_filter,
            matched,
            changed_paths,
            diff,
            next,
            suggestions,
        })
    }

    pub fn agent_new(
        &self,
        selector: &str,
        file: Option<&str>,
        patches: bool,
    ) -> Result<AgentNewReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let branch = self.lane_branch(&lane)?;
        let reviewed = self.latest_agent_review_marker(&lane)?;
        let base_change = reviewed
            .as_ref()
            .map(|marker| marker.checkpoint.clone())
            .unwrap_or_else(|| branch.base_change.clone());
        let head_change = branch.head_change.clone();
        let file_filter = file
            .map(|path| self.normalize_agent_query_path(&view, path))
            .transpose()?;
        let diff = self.diff_refs_with_options(&base_change.0, &head_change.0, patches, false)?;
        let (diff, file_filter) = if let Some(path) = file_filter {
            (agent_filter_diff_to_path(diff, &path)?, Some(path))
        } else {
            (diff, None)
        };
        let changed_paths = diff.files.clone();
        let matched = file_filter.is_none() || !changed_paths.is_empty();
        let mut groups = self.agent_turn_change_groups(&view)?;
        if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
        }
        let new_groups = agent_groups_after_review_marker(groups, reviewed.as_ref(), &head_change);
        let status = agent_new_status(reviewed.as_ref(), &changed_paths);
        let next = agent_new_next(&lane, &status, file_filter.as_deref(), matched, patches);
        let suggestions = agent_new_suggestions(
            &lane,
            &status,
            file_filter.as_deref(),
            matched,
            patches,
            &changed_paths,
            &next,
        );
        let summary = agent_new_summary(
            &view.task,
            &status,
            reviewed.as_ref(),
            file_filter.as_deref(),
            matched,
            &changed_paths,
            patches,
        );
        let diff = Some(AgentDiffReport {
            task: view.task.clone(),
            target_kind: "reviewed".to_string(),
            target: reviewed
                .as_ref()
                .map(|marker| format!("since {}", marker.checkpoint.0))
                .unwrap_or_else(|| "whole task".to_string()),
            turn_id: None,
            operation_id: None,
            before_change: base_change.clone(),
            after_change: head_change.clone(),
            file_filter: file_filter.clone(),
            diff,
            suggestions: vec![
                StatusSuggestion {
                    command: format!("crabdb agent changes {lane}"),
                    reason: "review the full task grouped by high-level change cards".to_string(),
                },
                next.clone(),
            ],
        });
        Ok(AgentNewReport {
            task: view.task,
            lane,
            status,
            summary,
            reviewed,
            base_change,
            head_change,
            new_groups,
            file_filter,
            matched,
            changed_paths,
            diff,
            next,
            suggestions,
        })
    }

    pub fn agent_mark_reviewed(
        &mut self,
        selector: &str,
        note: Option<String>,
    ) -> Result<AgentMarkReviewedReport> {
        let _lock = self.acquire_write_lock()?;
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let branch = self.lane_branch(&lane)?;
        let previous = self.latest_agent_review_marker(&lane)?;
        let note = note.filter(|value| !value.trim().is_empty());
        let payload = serde_json::json!({
            "checkpoint": branch.head_change.0,
            "changed_paths": view.task.changed_paths.len(),
            "note": note
        });
        let event_id = self.insert_lane_event_with_context(
            &branch.lane_id,
            view.task.session_id.as_deref(),
            None,
            AGENT_REVIEWED_EVENT,
            Some(&branch.head_change),
            None,
            &payload,
        )?;
        let marker = self
            .agent_review_marker_from_event(self.lane_event(&event_id)?)?
            .ok_or_else(|| {
                Error::InvalidInput("failed to read stored agent review marker".to_string())
            })?;
        let summary = agent_mark_reviewed_summary(&view.task, previous.as_ref(), &marker);
        let suggestions = agent_mark_reviewed_suggestions(&lane);
        Ok(AgentMarkReviewedReport {
            task: view.task,
            lane,
            marker,
            previous,
            summary,
            suggestions,
        })
    }

    pub fn agent_change_set(
        &self,
        selector: &str,
        change_selector: &str,
        patches: bool,
    ) -> Result<AgentChangeSetReport> {
        let changes = self.agent_changes(selector, false)?;
        let card = agent_select_change_card(&changes.cards, change_selector)?.clone();
        let groups = changes
            .groups
            .iter()
            .filter(|group| agent_group_touches_any_path(group, &card.changed_paths))
            .cloned()
            .collect::<Vec<_>>();
        let files = card
            .changed_paths
            .iter()
            .map(|change| agent_file_entry(&changes.lane, change, &groups))
            .collect::<Vec<_>>();
        let diffs = if patches {
            let mut diffs = Vec::new();
            for file in &files {
                diffs.push(self.agent_change_set_diff_for_file(&changes.lane, file)?);
            }
            diffs
        } else {
            Vec::new()
        };
        let next = if patches {
            StatusSuggestion {
                command: format!("crabdb agent review-plan {}", changes.lane),
                reason: "return to the task review dashboard after inspecting this change set"
                    .to_string(),
            }
        } else {
            StatusSuggestion {
                command: format!("crabdb agent change {} {} --patch", changes.lane, card.key),
                reason: "show focused patches for this change set".to_string(),
            }
        };
        let suggestions = agent_change_set_suggestions(&changes.lane, &card, &next, patches);
        let summary = agent_change_set_summary(&card, &groups, &files, patches);
        Ok(AgentChangeSetReport {
            task: changes.task,
            lane: changes.lane,
            selector: change_selector.to_string(),
            summary,
            card,
            groups,
            files,
            diffs,
            next,
            suggestions,
        })
    }

    pub fn agent_timeline(
        &self,
        selector: &str,
        by_operation: bool,
    ) -> Result<AgentTimelineReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let branch = self.lane_branch(&lane)?;
        let mut groups = if by_operation {
            self.agent_operation_change_groups(&view)?
        } else {
            self.agent_turn_change_groups(&view)?
        };
        let mode = if by_operation {
            "operation"
        } else if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
            "operation"
        } else {
            "turn"
        };
        let items = agent_timeline_items(&lane, &view, &groups);
        let summary = agent_timeline_summary(&view, mode, &items);
        let suggestions = agent_timeline_suggestions(&lane, mode, &items);
        Ok(AgentTimelineReport {
            task: view.task,
            lane,
            mode: mode.to_string(),
            summary,
            base_change: branch.base_change,
            head_change: branch.head_change,
            items,
            suggestions,
        })
    }

    fn agent_change_set_diff_for_file(
        &self,
        lane: &str,
        file: &AgentFileEntry,
    ) -> Result<AgentDiffReport> {
        let Some(touch) = file.touched_by.first() else {
            return self.agent_diff(lane, None, None, None, false, Some(&file.change.path), true);
        };
        match touch.kind.as_str() {
            "turn" => self.agent_diff(
                lane,
                Some(&touch.index.to_string()),
                None,
                None,
                false,
                Some(&file.change.path),
                true,
            ),
            _ => {
                if let Some(operation) = &touch.operation_id {
                    self.agent_diff(
                        lane,
                        None,
                        Some(&operation.0),
                        None,
                        false,
                        Some(&file.change.path),
                        true,
                    )
                } else {
                    self.agent_diff(lane, None, None, None, false, Some(&file.change.path), true)
                }
            }
        }
    }

    pub fn agent_why(&self, selector: &str, path: &str) -> Result<AgentWhyReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let path = self.normalize_agent_query_path(&view, path)?;
        let task_change = view
            .task
            .changed_paths
            .iter()
            .find(|file| agent_file_matches_path(file, &path))
            .cloned();

        let turn_groups = self.agent_turn_change_groups(&view)?;
        let mut groups = filter_agent_groups_by_path(turn_groups.clone(), &path);
        if groups.is_empty() {
            let operation_groups = self.agent_operation_change_groups(&view)?;
            let operation_matches = filter_agent_groups_by_path(operation_groups, &path);
            if !operation_matches.is_empty() || turn_groups.is_empty() {
                groups = operation_matches;
            }
        }

        let matched = !groups.is_empty();
        let summary = agent_why_summary(&view.task, &path, &groups);
        let mut suggestions = Vec::new();
        if let Some(group) = groups.first() {
            match group.kind.as_str() {
                "turn" => agent_push_suggestion(
                    &mut suggestions,
                    agent_turn_diff_command(&lane, Some(group.index), Some(&path), true),
                    "inspect the file patch for the turn that changed it",
                ),
                _ => {
                    if let Some(operation) = &group.operation_id {
                        agent_push_suggestion(
                            &mut suggestions,
                            format!(
                                "crabdb agent diff {lane} --operation {} --file {} --patch",
                                operation.0,
                                agent_shell_arg(&path)
                            ),
                            "inspect the file patch for the operation that changed it",
                        );
                    }
                }
            }
        }
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent changes {lane}"),
            "see all prompt-to-checkpoint changes for this task",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent review-plan {lane}"),
            "review readiness, blockers, transcript, and changed paths",
        );

        Ok(AgentWhyReport {
            task: view.task,
            path,
            matched,
            summary,
            task_change,
            groups,
            suggestions,
        })
    }

    pub fn agent_turn(
        &self,
        selector: &str,
        turn_selector: &str,
        file: Option<&str>,
        patches: bool,
    ) -> Result<AgentTurnReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let (
            index,
            id,
            turn_id,
            status,
            prompt_preview,
            assistant_preview,
            checkpoint,
            before_change,
            after_change,
            messages,
            event_count,
            tool_summaries,
        ) = {
            let (index, turn) = agent_turn_by_user_selector(&view, turn_selector)?;
            let checkpoint = turn
                .checkpoint
                .clone()
                .or_else(|| turn.turn.after_change.clone());
            (
                index,
                turn.turn.turn_id.clone(),
                turn.turn.turn_id.clone(),
                turn.turn.status.clone(),
                turn_prompt_preview(turn),
                turn_assistant_preview(turn),
                checkpoint.clone(),
                turn.turn.before_change.clone(),
                checkpoint,
                turn.messages.clone(),
                turn.events.len(),
                turn.tool_summaries.clone(),
            )
        };

        let diff = if after_change.is_some() {
            Some(self.agent_diff(
                &lane,
                Some(&index.to_string()),
                None,
                None,
                false,
                file,
                patches,
            )?)
        } else {
            if file.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{turn_selector}` has no checkpoint to filter by file"
                )));
            }
            None
        };
        let changed_paths = diff
            .as_ref()
            .map(|diff| diff.diff.files.clone())
            .unwrap_or_default();
        let suggestions = agent_turn_suggestions(&lane, index, file, patches, &changed_paths);
        Ok(AgentTurnReport {
            task: view.task,
            index,
            id,
            turn_id,
            status,
            prompt_preview,
            assistant_preview,
            checkpoint,
            before_change,
            after_change,
            changed_paths,
            tool_summaries,
            messages,
            event_count,
            diff,
            suggestions,
        })
    }

    pub fn agent_diff(
        &self,
        selector: &str,
        turn: Option<&str>,
        operation: Option<&str>,
        checkpoint: Option<&str>,
        last_turn: bool,
        file: Option<&str>,
        patches: bool,
    ) -> Result<AgentDiffReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let branch = self.lane_branch(&lane)?;
        let target_count = usize::from(turn.is_some())
            + usize::from(operation.is_some())
            + usize::from(checkpoint.is_some())
            + usize::from(last_turn);
        if target_count > 1 {
            return Err(Error::InvalidInput(
                "agent diff accepts only one of --turn, --operation, --checkpoint, or --last-turn"
                    .to_string(),
            ));
        }

        let target = if let Some(turn_selector) = turn {
            self.agent_diff_target_for_turn(&view, turn_selector)?
        } else if last_turn {
            self.agent_diff_target_for_last_turn(&view)?
        } else if let Some(change_id) = operation {
            self.agent_diff_target_for_operation(&view, change_id, "operation")?
        } else if let Some(change_id) = checkpoint {
            self.agent_diff_target_for_checkpoint(&view, change_id)?
        } else {
            AgentResolvedDiffTarget {
                target_kind: "task".to_string(),
                target: "whole task".to_string(),
                turn_id: None,
                operation_id: None,
                before_change: branch.base_change,
                after_change: branch.head_change,
            }
        };

        let diff = self.diff_refs_with_options(
            &target.before_change.0,
            &target.after_change.0,
            patches,
            false,
        )?;
        let (diff, file_filter) = if let Some(file) = file {
            let path = self.normalize_agent_query_path(&view, file)?;
            (agent_filter_diff_to_path(diff, &path)?, Some(path))
        } else {
            (diff, None)
        };
        let mut suggestions = vec![StatusSuggestion {
            command: format!("crabdb agent changes {lane}"),
            reason: "return to the prompt-to-checkpoint change summary".to_string(),
        }];
        if let Some(path) = &file_filter {
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent why {lane} {}", agent_shell_arg(path)),
                "explain which prompt, turn, tools, and checkpoint changed this file",
            );
        }
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent land {lane} --dry-run"),
            "preview applying the whole agent task",
        );
        Ok(AgentDiffReport {
            task: view.task,
            target_kind: target.target_kind,
            target: target.target,
            turn_id: target.turn_id,
            operation_id: target.operation_id,
            before_change: target.before_change,
            after_change: target.after_change,
            file_filter,
            diff,
            suggestions,
        })
    }

    pub fn agent_apply(
        &mut self,
        selector: &str,
        dry_run: bool,
        message: Option<String>,
    ) -> Result<AgentApplyReport> {
        let lane = self
            .resolve_agent_selector(selector)?
            .ok_or_else(|| Error::InvalidInput("no agent tasks have been recorded".to_string()))?;
        let crab_branch = self.config.workspace.default_branch.clone();
        let target_ref_name = branch_ref(&crab_branch);
        let target_ref = self.get_ref(&target_ref_name)?;
        let git_branch = self.current_git_branch()?;
        let git_state = self.current_git_state()?.ok_or_else(|| {
            Error::Git(format!(
                "agent apply requires a Git working tree at {}; CrabDB branch `{crab_branch}` is the internal apply base",
                self.workspace_root.display()
            ))
        })?;
        if git_state.dirty {
            return Err(Error::Git(
                "current Git worktree has tracked changes; commit, stash, or revert them before `crabdb agent apply`".to_string(),
            ));
        }
        self.ensure_git_head_matches_root(
            &target_ref.root_id,
            git_state.head.as_deref(),
            &crab_branch,
        )?;

        let would_record = self.lane_workdir_dirty(&lane)?;
        if dry_run && would_record {
            let view = self.agent_task_view(&lane)?;
            let plan = AgentGitApplyPlan {
                crab_branch,
                git_branch,
                base_change: target_ref.change_id,
                result_change: None,
                range: None,
                would_record: true,
                would_create_git_commit: false,
                would_fast_forward: false,
            };
            return Ok(AgentApplyReport {
                task: view.task,
                status: "would_record".to_string(),
                dry_run,
                git_apply_plan: plan,
                recorded: None,
                merge: None,
                git_export: None,
                fast_forwarded: false,
                warnings: vec![
                    "lane workdir has unrecorded changes; actual apply will record them first"
                        .to_string(),
                ],
                suggestions: vec![StatusSuggestion {
                    command: format!("crabdb agent land {lane}"),
                    reason: "record the lane workdir and apply the agent task".to_string(),
                }],
            });
        }

        let recorded = if would_record {
            Some(self.record_lane_workdir(&lane, Some(format!("Agent task `{lane}` checkpoint")))?)
        } else {
            None
        };

        let merge = self.merge_lane_user_with_options(&lane, &crab_branch, dry_run, true)?;
        let range = if merge.changed_paths.is_empty() {
            None
        } else {
            Some(format!("{}..{}", target_ref.change_id.0, merge.operation.0))
        };
        let plan = AgentGitApplyPlan {
            crab_branch: crab_branch.clone(),
            git_branch: git_branch.clone(),
            base_change: target_ref.change_id.clone(),
            result_change: range.as_ref().map(|_| merge.operation.clone()),
            range: range.clone(),
            would_record,
            would_create_git_commit: range.is_some(),
            would_fast_forward: range.is_some(),
        };
        if dry_run {
            let view = self.agent_task_view(&lane)?;
            let (status, suggestions) = if merge.conflicts.is_empty() {
                (
                    "ready".to_string(),
                    vec![StatusSuggestion {
                        command: format!("crabdb agent land {lane}"),
                        reason: format!(
                            "create a Git commit using default message `{}` and fast-forward the current branch",
                            default_agent_apply_message_for_task(&view.task)
                        ),
                    }],
                )
            } else {
                (
                    "conflicted".to_string(),
                    vec![StatusSuggestion {
                        command: format!("crabdb agent view {lane}"),
                        reason: "inspect merge conflicts before applying".to_string(),
                    }],
                )
            };
            return Ok(AgentApplyReport {
                task: view.task,
                status,
                dry_run,
                git_apply_plan: plan,
                recorded,
                merge: Some(merge),
                git_export: None,
                fast_forwarded: false,
                warnings: Vec::new(),
                suggestions,
            });
        }

        let git_export = if let Some(range) = &range {
            let view = self.agent_task_view(&lane)?;
            let message = message
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| default_agent_apply_message_for_task(&view.task));
            Some(self.git_export_commit(range, &message)?)
        } else {
            None
        };
        if let Some(export) = &git_export {
            self.git_fast_forward(&export.commit)?;
        }
        let view = self.agent_task_view(&lane)?;
        Ok(AgentApplyReport {
            task: view.task,
            status: if git_export.is_some() {
                "applied".to_string()
            } else {
                "already_applied".to_string()
            },
            dry_run,
            git_apply_plan: plan,
            recorded,
            merge: Some(merge),
            git_export,
            fast_forwarded: range.is_some(),
            warnings: Vec::new(),
            suggestions: vec![StatusSuggestion {
                command: format!("crabdb agent view {lane}"),
                reason: "inspect the applied task transcript and checkpoint".to_string(),
            }],
        })
    }

    pub fn agent_rewind(&mut self, selector: &str, target: &str) -> Result<LaneRewindReport> {
        let lane = self
            .resolve_agent_selector(selector)?
            .ok_or_else(|| Error::InvalidInput("no agent tasks have been recorded".to_string()))?;
        let target = self.resolve_agent_rewind_target(&lane, target)?;
        self.rewind_lane(&lane, &target, true, true)
    }

    pub fn agent_undo(
        &mut self,
        selector: &str,
        last_turn: bool,
        turn: Option<&str>,
        prompt: Option<&str>,
        last_operation: bool,
    ) -> Result<LaneRewindReport> {
        let modes = usize::from(last_turn)
            + usize::from(turn.is_some())
            + usize::from(prompt.is_some())
            + usize::from(last_operation);
        if modes > 1 {
            return Err(Error::InvalidInput(
                "agent undo accepts only one of --last-turn, --turn, --prompt, or --last-operation"
                    .to_string(),
            ));
        }
        let target = if let Some(turn) = turn {
            let turn = turn.trim();
            if turn.is_empty() {
                return Err(Error::InvalidInput(
                    "agent undo --turn cannot be empty".to_string(),
                ));
            }
            format!("before-turn:{turn}")
        } else if let Some(prompt) = prompt {
            let prompt = prompt.trim();
            if prompt.is_empty() {
                return Err(Error::InvalidInput(
                    "agent undo --prompt cannot be empty".to_string(),
                ));
            }
            format!("before-prompt:{prompt}")
        } else if last_operation {
            "before-last-operation".to_string()
        } else {
            "before-last-turn".to_string()
        };
        self.agent_rewind(selector, &target)
    }

    pub(crate) fn agent_task_for_lane_details(
        &self,
        lane: LaneDetails,
        limit: usize,
    ) -> Result<AgentTaskReport> {
        let review = self.lane_review_packet(&lane.record.name, limit)?;
        let transcript = self.transcript(&lane.record.name).ok();
        self.agent_task_from_review(&review, transcript.as_ref())
    }

    fn agent_task_from_review(
        &self,
        review: &LaneReviewPacketReport,
        transcript: Option<&TranscriptReport>,
    ) -> Result<AgentTaskReport> {
        let lane_name = review.lane.record.name.clone();
        let acp_session = self
            .list_lane_acp_sessions(Some(&lane_name))?
            .sessions
            .into_iter()
            .next();
        let session_id = acp_session
            .as_ref()
            .map(|session| session.crabdb_session_id.clone())
            .or_else(|| {
                review
                    .recent_sessions
                    .first()
                    .map(|session| session.session_id.clone())
            });
        let latest_checkpoint = transcript
            .and_then(|report| {
                report
                    .turns
                    .iter()
                    .rev()
                    .find_map(|turn| turn.checkpoint.clone())
            })
            .or_else(|| {
                review
                    .recent_operations
                    .iter()
                    .find(|operation| {
                        matches!(
                            operation.kind,
                            OperationKind::LaneRecord
                                | OperationKind::LanePatch
                                | OperationKind::LaneMerge
                                | OperationKind::LaneRewind
                        )
                    })
                    .map(|operation| operation.change_id.clone())
            });
        let turns = transcript
            .map(|report| report.turns.len())
            .unwrap_or(review.recent_sessions.len());
        let tool_events = transcript
            .map(|report| {
                report
                    .turns
                    .iter()
                    .map(|turn| turn.tool_summaries.len())
                    .sum()
            })
            .unwrap_or_else(|| {
                review
                    .recent_events
                    .iter()
                    .filter(|event| {
                        matches!(
                            event.event_type.as_str(),
                            "tool_call" | "tool_call_update" | "acp_available_commands_update"
                        )
                    })
                    .count()
            });
        let status = agent_status_from_review(review);
        let suggestions = agent_task_suggestions(&lane_name, &status);
        let title = agent_task_title(
            &lane_name,
            review.lane.record.provider.as_deref(),
            transcript,
        );
        Ok(AgentTaskReport {
            task_id: lane_name.clone(),
            name: lane_name.clone(),
            title,
            provider: review.lane.record.provider.clone(),
            editor: None,
            lane: lane_name,
            workdir: review.lane.branch.workdir.clone(),
            session_id,
            acp_session_id: acp_session.map(|session| session.acp_session_id),
            status,
            changed_paths: review.changed_paths.clone(),
            latest_checkpoint,
            turns,
            tool_events,
            suggestions,
        })
    }

    fn resolve_agent_selector(&self, selector: &str) -> Result<Option<String>> {
        if selector == "latest" {
            return self.latest_agent_lane();
        }
        if let Ok(details) = self.lane_details(selector) {
            return Ok(Some(details.record.name));
        }
        if let Some(acp) = self.try_lane_acp_session(selector)? {
            return self.resolve_lane_handle(&acp.lane_id).map(Some);
        }
        if let Some(session) = self.try_lane_session(selector)? {
            return self.resolve_lane_handle(&session.lane_id).map(Some);
        }
        Err(Error::RefNotFound(selector.to_string()))
    }

    fn latest_agent_lane(&self) -> Result<Option<String>> {
        if let Some(acp) = self
            .list_lane_acp_sessions(None)?
            .sessions
            .into_iter()
            .next()
        {
            return self.resolve_lane_handle(&acp.lane_id).map(Some);
        }
        let mut lanes = self.list_lanes()?;
        lanes.sort_by(|left, right| {
            right
                .branch
                .updated_at
                .cmp(&left.branch.updated_at)
                .then_with(|| right.record.created_at.cmp(&left.record.created_at))
        });
        for lane in lanes {
            if self.lane_looks_like_agent_task(&lane)? {
                return Ok(Some(lane.record.name));
            }
        }
        Ok(None)
    }

    fn lane_looks_like_agent_task(&self, lane: &LaneDetails) -> Result<bool> {
        if lane.record.provider.is_some() || lane.record.name.starts_with("agent-") {
            return Ok(true);
        }
        Ok(!self
            .list_lane_acp_sessions(Some(&lane.record.name))?
            .sessions
            .is_empty())
    }

    fn agent_turn_change_groups(
        &self,
        view: &AgentTaskViewReport,
    ) -> Result<Vec<AgentChangeGroup>> {
        let Some(transcript) = &view.transcript else {
            return Ok(Vec::new());
        };
        let mut groups = Vec::new();
        for (idx, turn) in transcript.turns.iter().enumerate() {
            let after_change = turn
                .checkpoint
                .clone()
                .or_else(|| turn.turn.after_change.clone());
            let changed_paths = after_change
                .as_ref()
                .map(|after| self.diff_refs(&turn.turn.before_change.0, &after.0, false))
                .transpose()?
                .map(|diff| diff.files)
                .unwrap_or_default();
            groups.push(AgentChangeGroup {
                kind: "turn".to_string(),
                index: idx + 1,
                id: turn.turn.turn_id.clone(),
                turn_id: Some(turn.turn.turn_id.clone()),
                operation_id: after_change.clone(),
                operations: turn_operation_ids(turn),
                before_change: Some(turn.turn.before_change.clone()),
                after_change: after_change.clone(),
                checkpoint: after_change,
                status: Some(turn.turn.status.clone()),
                prompt_preview: turn_prompt_preview(turn),
                assistant_preview: turn_assistant_preview(turn),
                tool_summaries: turn.tool_summaries.clone(),
                changed_paths,
            });
        }
        Ok(groups)
    }

    fn agent_operation_change_groups(
        &self,
        view: &AgentTaskViewReport,
    ) -> Result<Vec<AgentChangeGroup>> {
        let operations = self.agent_operation_entries(view)?;
        let mut groups = Vec::new();
        for (idx, entry) in operations.into_iter().enumerate() {
            let operation = self.operation(&entry.change_id)?;
            let before_change = operation.parents.first().cloned();
            let changed_paths = before_change
                .as_ref()
                .map(|before| self.diff_refs(&before.0, &entry.change_id.0, false))
                .transpose()?
                .map(|diff| diff.files)
                .unwrap_or_default();
            let turn = view.transcript.as_ref().and_then(|transcript| {
                transcript
                    .turns
                    .iter()
                    .find(|turn| turn_contains_operation(turn, &entry.change_id))
            });
            groups.push(AgentChangeGroup {
                kind: "operation".to_string(),
                index: idx + 1,
                id: entry.change_id.0.clone(),
                turn_id: turn.map(|turn| turn.turn.turn_id.clone()),
                operation_id: Some(entry.change_id.clone()),
                operations: vec![entry.change_id.clone()],
                before_change,
                after_change: Some(entry.change_id.clone()),
                checkpoint: Some(entry.change_id.clone()),
                status: Some(format!("{:?}", entry.kind)),
                prompt_preview: turn.and_then(turn_prompt_preview),
                assistant_preview: turn.and_then(turn_assistant_preview),
                tool_summaries: turn
                    .map(|turn| turn.tool_summaries.clone())
                    .unwrap_or_default(),
                changed_paths,
            });
        }
        Ok(groups)
    }

    fn agent_operation_entries(&self, view: &AgentTaskViewReport) -> Result<Vec<TimelineEntry>> {
        if let Some(transcript) = &view.transcript {
            return Ok(transcript.operations.clone());
        }
        let mut operations = self.lane_timeline(&view.task.lane, 200)?;
        operations.reverse();
        Ok(operations)
    }

    fn agent_next_report_for_view(&self, view: &AgentTaskViewReport) -> Result<AgentNextReport> {
        let progress = if view.task.status == AgentTaskStatus::Ready {
            Some(self.agent_review_progress_for_view(view)?)
        } else {
            None
        };
        Ok(agent_next_report_from_view(view, progress.as_ref()))
    }

    fn agent_inbox_next_for_task(&self, task: &AgentTaskReport) -> Result<StatusSuggestion> {
        if task.status == AgentTaskStatus::Ready {
            let view = self.agent_task_view(&task.lane)?;
            return Ok(self.agent_next_report_for_view(&view)?.primary);
        }
        Ok(agent_inbox_next_for_task(task))
    }

    fn agent_inbox_item_for_task(&self, task: &AgentTaskReport) -> Result<AgentInboxItem> {
        let mut next = self.agent_inbox_next_for_task(task)?;
        let mut suggestions = vec![next.clone()];
        let mut new_changed_paths = 0;
        let mut new_changed_lines = 0;
        let mut review_first = None;
        let (attention, detail) = match &task.status {
            AgentTaskStatus::Empty => (
                "setup".to_string(),
                "No agent task is available yet.".to_string(),
            ),
            AgentTaskStatus::Active => (
                "active".to_string(),
                "Agent task is active or has no recorded checkpoint yet.".to_string(),
            ),
            AgentTaskStatus::Dirty => (
                "record_needed".to_string(),
                "Materialized agent workdir has unrecorded changes.".to_string(),
            ),
            AgentTaskStatus::Blocked => (
                "blocked".to_string(),
                "Readiness blockers require review before apply.".to_string(),
            ),
            AgentTaskStatus::Conflicted => (
                "conflicted".to_string(),
                "Task has conflicts and cannot be applied safely yet.".to_string(),
            ),
            AgentTaskStatus::Applied => (
                "applied".to_string(),
                "Task has already been applied.".to_string(),
            ),
            AgentTaskStatus::Ready => {
                let view = self.agent_task_view(&task.lane)?;
                let progress = self.agent_review_progress_for_view(&view)?;
                new_changed_paths = progress.changed_paths;
                new_changed_lines = progress.changed_lines;
                let review = self.agent_review(&task.lane)?;
                if let Some(priority) = review.priorities.first() {
                    review_first = Some(AgentInboxReviewTarget {
                        path: priority.change.path.clone(),
                        reason: priority
                            .reasons
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "highest-ranked review priority".to_string()),
                        command: format!(
                            "crabdb agent focus {} --file {} --patch",
                            task.lane,
                            agent_shell_arg(&priority.change.path)
                        ),
                    });
                }
                let next_report = self.agent_next_report_for_view(&view)?;
                next = next_report.primary;
                suggestions = vec![next.clone()];
                for suggestion in next_report.suggestions {
                    agent_push_suggestion(&mut suggestions, suggestion.command, &suggestion.reason);
                }
                if let Some(target) = &review_first {
                    agent_push_suggestion(
                        &mut suggestions,
                        target.command.clone(),
                        "start with the highest-priority file and patch",
                    );
                }
                let detail = match progress.status.as_str() {
                    "up_to_date" if progress.reviewed.is_some() => {
                        "Current checkpoint has already been marked reviewed.".to_string()
                    }
                    "new_changes" => format!(
                        "{} changed file(s), {} changed line(s) since the last review.",
                        progress.changed_paths, progress.changed_lines
                    ),
                    "unreviewed" => format!(
                        "{} changed file(s), {} changed line(s) have not been reviewed yet.",
                        progress.changed_paths, progress.changed_lines
                    ),
                    _ => "Ready for human review before apply.".to_string(),
                };
                (progress.status, detail)
            }
        };
        Ok(AgentInboxItem {
            task: task.clone(),
            attention,
            detail,
            new_changed_paths,
            new_changed_lines,
            review_first,
            next,
            suggestions,
        })
    }

    fn agent_review_progress_for_view(
        &self,
        view: &AgentTaskViewReport,
    ) -> Result<AgentReviewProgress> {
        let branch = self.lane_branch(&view.task.lane)?;
        let reviewed = self.latest_agent_review_marker(&view.task.lane)?;
        let base_change = reviewed
            .as_ref()
            .map(|marker| marker.checkpoint.clone())
            .unwrap_or_else(|| branch.base_change.clone());
        let diff =
            self.diff_refs_with_options(&base_change.0, &branch.head_change.0, false, true)?;
        let changed_lines = diff
            .files
            .iter()
            .map(|file| file.additions + file.deletions)
            .sum();
        let status = agent_new_status(reviewed.as_ref(), &diff.files);
        Ok(AgentReviewProgress {
            status,
            reviewed,
            changed_paths: diff.files.len(),
            changed_lines,
        })
    }

    fn latest_agent_review_marker(&self, lane: &str) -> Result<Option<AgentReviewMarker>> {
        let branch = self.lane_branch(lane)?;
        let mut stmt = self.conn.prepare(
            "SELECT event_id, lane_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM lane_events \
             WHERE lane_id = ?1 AND event_type = ?2 \
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )?;
        let event = stmt
            .query_row(
                params![branch.lane_id, AGENT_REVIEWED_EVENT],
                lane_event_row,
            )
            .optional()?;
        event
            .map(|event| self.agent_review_marker_from_event(event))
            .transpose()
            .map(Option::flatten)
    }

    fn agent_review_marker_from_event(
        &self,
        event: LaneEventRecord,
    ) -> Result<Option<AgentReviewMarker>> {
        if event.event_type != AGENT_REVIEWED_EVENT {
            return Ok(None);
        }
        let payload = event.payload.as_ref();
        let checkpoint = event
            .change_id
            .clone()
            .or_else(|| {
                payload
                    .and_then(|payload| payload.get("checkpoint"))
                    .and_then(serde_json::Value::as_str)
                    .map(|value| ChangeId(value.to_string()))
            })
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "agent reviewed marker `{}` is missing checkpoint",
                    event.event_id
                ))
            })?;
        let changed_paths = payload
            .and_then(|payload| payload.get("changed_paths"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;
        let note = payload
            .and_then(|payload| payload.get("note"))
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        Ok(Some(AgentReviewMarker {
            event_id: event.event_id,
            checkpoint,
            reviewed_at: event.created_at,
            changed_paths,
            note,
        }))
    }

    fn agent_latest_group_diff(&self, groups: &[AgentChangeGroup]) -> Result<Option<DiffSummary>> {
        let Some(group) = groups
            .iter()
            .rev()
            .find(|group| group.before_change.is_some() && group.after_change.is_some())
        else {
            return Ok(None);
        };
        let before = group.before_change.as_ref().expect("checked before_change");
        let after = group.after_change.as_ref().expect("checked after_change");
        Ok(Some(self.diff_refs_with_options(
            &before.0, &after.0, false, false,
        )?))
    }

    fn agent_diff_target_for_turn(
        &self,
        view: &AgentTaskViewReport,
        selector: &str,
    ) -> Result<AgentResolvedDiffTarget> {
        let transcript = view.transcript.as_ref().ok_or_else(|| {
            Error::InvalidInput(format!(
                "agent task `{}` has no turn transcript",
                view.task.name
            ))
        })?;
        let turn = if let Ok(index) = selector.parse::<usize>() {
            if index == 0 {
                return Err(Error::InvalidInput(
                    "turn index is 1-based; use --turn 1 for the first turn".to_string(),
                ));
            }
            transcript.turns.get(index - 1)
        } else {
            transcript
                .turns
                .iter()
                .find(|turn| turn.turn.turn_id == selector)
        }
        .ok_or_else(|| Error::InvalidInput(format!("turn `{selector}` not found")))?;
        self.agent_diff_target_from_turn(turn, "turn", selector.to_string())
    }

    fn agent_diff_target_for_last_turn(
        &self,
        view: &AgentTaskViewReport,
    ) -> Result<AgentResolvedDiffTarget> {
        let transcript = view.transcript.as_ref().ok_or_else(|| {
            Error::InvalidInput(format!(
                "agent task `{}` has no turn transcript",
                view.task.name
            ))
        })?;
        let turn = transcript
            .turns
            .iter()
            .rev()
            .find(|turn| turn.checkpoint.is_some() || turn.turn.after_change.is_some())
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "agent task `{}` has no completed turn checkpoint",
                    view.task.name
                ))
            })?;
        self.agent_diff_target_from_turn(turn, "turn", "last turn".to_string())
    }

    fn agent_diff_target_from_turn(
        &self,
        turn: &TranscriptTurn,
        target_kind: &str,
        target: String,
    ) -> Result<AgentResolvedDiffTarget> {
        let after_change = turn
            .checkpoint
            .clone()
            .or_else(|| turn.turn.after_change.clone())
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "turn `{}` has no checkpoint to diff",
                    turn.turn.turn_id
                ))
            })?;
        Ok(AgentResolvedDiffTarget {
            target_kind: target_kind.to_string(),
            target,
            turn_id: Some(turn.turn.turn_id.clone()),
            operation_id: Some(after_change.clone()),
            before_change: turn.turn.before_change.clone(),
            after_change,
        })
    }

    fn agent_diff_target_for_operation(
        &self,
        view: &AgentTaskViewReport,
        change_id: &str,
        target_kind: &str,
    ) -> Result<AgentResolvedDiffTarget> {
        let change = ChangeId(change_id.to_string());
        if !self
            .agent_operation_entries(view)?
            .iter()
            .any(|entry| entry.change_id == change)
        {
            return Err(Error::InvalidInput(format!(
                "operation `{change_id}` is not part of agent task `{}`",
                view.task.name
            )));
        }
        let operation = self.operation(&change)?;
        let before_change = operation.parents.first().cloned().ok_or_else(|| {
            Error::InvalidInput(format!(
                "operation `{change_id}` has no parent change to diff"
            ))
        })?;
        let turn_id = view.transcript.as_ref().and_then(|transcript| {
            transcript
                .turns
                .iter()
                .find(|turn| turn_contains_operation(turn, &change))
                .map(|turn| turn.turn.turn_id.clone())
        });
        Ok(AgentResolvedDiffTarget {
            target_kind: target_kind.to_string(),
            target: change_id.to_string(),
            turn_id,
            operation_id: Some(change.clone()),
            before_change,
            after_change: change,
        })
    }

    fn agent_diff_target_for_checkpoint(
        &self,
        view: &AgentTaskViewReport,
        change_id: &str,
    ) -> Result<AgentResolvedDiffTarget> {
        if let Some(transcript) = &view.transcript {
            if let Some(turn) = transcript.turns.iter().find(|turn| {
                turn.checkpoint
                    .as_ref()
                    .or(turn.turn.after_change.as_ref())
                    .is_some_and(|checkpoint| checkpoint.0 == change_id)
            }) {
                return self.agent_diff_target_from_turn(turn, "checkpoint", change_id.to_string());
            }
        }
        self.agent_diff_target_for_operation(view, change_id, "checkpoint")
    }

    fn resolve_agent_rewind_target(&self, lane: &str, target: &str) -> Result<String> {
        let target = target.trim();
        if target.is_empty() {
            return Err(Error::InvalidInput(
                "agent rewind target cannot be empty".to_string(),
            ));
        }
        let normalized = target.to_ascii_lowercase();
        if !agent_rewind_target_needs_resolution(&normalized) {
            return Ok(target.to_string());
        }

        let view = self.agent_task_view(lane)?;
        match normalized.as_str() {
            "base" | "task-start" | "before-task" => {
                return Ok(self.lane_branch(lane)?.base_change.0);
            }
            "last-turn" | "latest-turn" | "last-checkpoint" | "latest-checkpoint" => {
                return Ok(match agent_last_completed_turn(&view) {
                    Ok(turn) => agent_turn_checkpoint(turn)?.0,
                    Err(_) => self.agent_latest_operation_change(&view)?.0,
                });
            }
            "before-last-turn" | "before-latest-turn" | "previous-turn" => {
                return Ok(match agent_last_completed_turn(&view) {
                    Ok(turn) => turn.turn.before_change.0.clone(),
                    Err(_) => self.agent_latest_operation_parent(&view)?.0,
                });
            }
            "last-operation" | "latest-operation" => {
                return Ok(self.agent_latest_operation_change(&view)?.0);
            }
            "before-last-operation" | "before-latest-operation" | "previous-operation" => {
                return Ok(self.agent_latest_operation_parent(&view)?.0);
            }
            _ => {}
        }

        if let Some(selector) = strip_agent_alias_prefix(target, &["turn:", "turn "]) {
            return Ok(agent_turn_checkpoint(agent_turn_by_selector(&view, selector)?)?.0);
        }
        if let Some(selector) = strip_agent_alias_prefix(target, &["after-turn:", "after-turn "]) {
            return Ok(agent_turn_checkpoint(agent_turn_by_selector(&view, selector)?)?.0);
        }
        if let Some(selector) = strip_agent_alias_prefix(target, &["before-turn:", "before-turn "])
        {
            return Ok(agent_turn_by_selector(&view, selector)?
                .turn
                .before_change
                .0
                .clone());
        }
        if let Some(prompt) = strip_agent_alias_prefix(target, &["prompt:", "prompt "]) {
            return Ok(agent_turn_checkpoint(agent_turn_by_prompt(&view, prompt)?)?.0);
        }
        if let Some(prompt) =
            strip_agent_alias_prefix(target, &["before-prompt:", "before-prompt "])
        {
            return Ok(agent_turn_by_prompt(&view, prompt)?
                .turn
                .before_change
                .0
                .clone());
        }

        Ok(target.to_string())
    }

    fn agent_latest_operation_change(&self, view: &AgentTaskViewReport) -> Result<ChangeId> {
        self.agent_operation_entries(view)?
            .last()
            .map(|entry| entry.change_id.clone())
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "agent task `{}` has no recorded operations",
                    view.task.name
                ))
            })
    }

    fn agent_latest_operation_parent(&self, view: &AgentTaskViewReport) -> Result<ChangeId> {
        let change = self.agent_latest_operation_change(view)?;
        self.operation(&change)?
            .parents
            .first()
            .cloned()
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "operation `{}` has no parent change to rewind to",
                    change.0
                ))
            })
    }

    fn lane_workdir_dirty(&self, lane: &str) -> Result<bool> {
        let branch = self.lane_branch(lane)?;
        let head = self.get_ref(&branch.ref_name)?;
        Ok(self
            .lane_workdir_changed_paths(&branch, &head)?
            .is_some_and(|paths| !paths.is_empty()))
    }

    fn normalize_agent_query_path(&self, view: &AgentTaskViewReport, path: &str) -> Result<String> {
        let path = strip_agent_path_line_suffix(path.trim()).trim();
        if path.is_empty() {
            return Err(Error::InvalidInput(
                "agent why path cannot be empty".to_string(),
            ));
        }
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            if let Some(workdir) = &view.task.workdir {
                if let Ok(relative) = candidate.strip_prefix(Path::new(workdir)) {
                    return normalize_relative_path(&relative.to_string_lossy());
                }
            }
            if let Ok(relative) = candidate.strip_prefix(&self.workspace_root) {
                return normalize_relative_path(&relative.to_string_lossy());
            }
            return Err(Error::InvalidPath {
                path: path.to_string(),
                reason: format!(
                    "absolute path must be inside the workspace or task workdir `{}`",
                    view.task.lane
                ),
            });
        }
        let normalized = normalize_relative_path(path)?;
        let materialized_prefix = format!(".crabdb/worktrees/{}/", view.task.lane);
        if let Some(stripped) = normalized.strip_prefix(&materialized_prefix) {
            return normalize_relative_path(stripped);
        }
        Ok(normalized)
    }

    fn current_git_branch(&self) -> Result<Option<String>> {
        if self.current_git_state()?.is_none() {
            return Ok(None);
        }
        let branch = self.git_output(&["branch".to_string(), "--show-current".to_string()])?;
        Ok((!branch.trim().is_empty()).then_some(branch))
    }

    fn ensure_git_head_matches_root(
        &self,
        root_id: &ObjectId,
        git_head: Option<&str>,
        crab_branch: &str,
    ) -> Result<()> {
        let Some(git_head) = git_head else {
            return Err(Error::Git(
                "agent apply requires a Git HEAD commit before it can fast-forward".to_string(),
            ));
        };
        let files = self.load_root_files(root_id)?;
        let crab_tree = self.git_write_tree(&files)?;
        let git_tree =
            self.git_output(&["rev-parse".to_string(), format!("{git_head}^{{tree}}")])?;
        if crab_tree == git_tree {
            return Ok(());
        }
        Err(Error::Git(format!(
            "current Git HEAD does not match CrabDB branch `{crab_branch}`; run `crabdb git import-update --branch {crab_branch}` or apply from a Git branch that matches CrabDB `{crab_branch}`"
        )))
    }

    fn git_fast_forward(&self, commit: &str) -> Result<()> {
        self.git_output(&[
            "merge".to_string(),
            "--ff-only".to_string(),
            commit.to_string(),
        ])?;
        Ok(())
    }
}

struct AgentResolvedDiffTarget {
    target_kind: String,
    target: String,
    turn_id: Option<String>,
    operation_id: Option<ChangeId>,
    before_change: ChangeId,
    after_change: ChangeId,
}

fn agent_status_from_review(review: &LaneReviewPacketReport) -> AgentTaskStatus {
    if review.lane.branch.status == "merged" {
        return AgentTaskStatus::Applied;
    }
    if review.lane.branch.status == "conflicted"
        || review
            .readiness
            .blockers
            .iter()
            .any(|issue| issue.code == "open_conflicts")
    {
        return AgentTaskStatus::Conflicted;
    }
    if review
        .readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "dirty_workdir")
    {
        return AgentTaskStatus::Dirty;
    }
    if !review.readiness.ready {
        return AgentTaskStatus::Blocked;
    }
    if !review.changed_paths.is_empty() || review.evidence_summary.operations > 0 {
        return AgentTaskStatus::Ready;
    }
    AgentTaskStatus::Active
}

fn turn_operation_ids(turn: &TranscriptTurn) -> Vec<ChangeId> {
    let mut ids = Vec::new();
    for event in &turn.events {
        let Some(change_id) = &event.change_id else {
            continue;
        };
        if !ids.iter().any(|existing| existing == change_id) {
            ids.push(change_id.clone());
        }
    }
    if let Some(checkpoint) = turn.checkpoint.as_ref().or(turn.turn.after_change.as_ref()) {
        if !ids.iter().any(|existing| existing == checkpoint) {
            ids.push(checkpoint.clone());
        }
    }
    ids
}

fn turn_contains_operation(turn: &TranscriptTurn, change_id: &ChangeId) -> bool {
    turn.checkpoint.as_ref() == Some(change_id)
        || turn.turn.after_change.as_ref() == Some(change_id)
        || turn
            .events
            .iter()
            .any(|event| event.change_id.as_ref() == Some(change_id))
}

fn agent_rewind_target_needs_resolution(normalized: &str) -> bool {
    matches!(
        normalized,
        "base"
            | "task-start"
            | "before-task"
            | "last-turn"
            | "latest-turn"
            | "last-checkpoint"
            | "latest-checkpoint"
            | "before-last-turn"
            | "before-latest-turn"
            | "previous-turn"
            | "last-operation"
            | "latest-operation"
            | "before-last-operation"
            | "before-latest-operation"
            | "previous-operation"
    ) || [
        "turn:",
        "turn ",
        "after-turn:",
        "after-turn ",
        "before-turn:",
        "before-turn ",
        "prompt:",
        "prompt ",
        "before-prompt:",
        "before-prompt ",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
}

fn agent_last_completed_turn(view: &AgentTaskViewReport) -> Result<&TranscriptTurn> {
    let transcript = view.transcript.as_ref().ok_or_else(|| {
        Error::InvalidInput(format!(
            "agent task `{}` has no turn transcript",
            view.task.name
        ))
    })?;
    transcript
        .turns
        .iter()
        .rev()
        .find(|turn| agent_turn_checkpoint(turn).is_ok())
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "agent task `{}` has no completed turn checkpoint",
                view.task.name
            ))
        })
}

fn agent_turn_by_user_selector<'a>(
    view: &'a AgentTaskViewReport,
    selector: &str,
) -> Result<(usize, &'a TranscriptTurn)> {
    let selector = selector.trim();
    let transcript = view.transcript.as_ref().ok_or_else(|| {
        Error::InvalidInput(format!(
            "agent task `{}` has no turn transcript",
            view.task.name
        ))
    })?;
    if matches!(
        selector,
        "" | "last" | "latest" | "last-turn" | "latest-turn"
    ) {
        return transcript
            .turns
            .iter()
            .enumerate()
            .rev()
            .find(|(_, turn)| agent_turn_checkpoint(turn).is_ok())
            .map(|(index, turn)| (index + 1, turn))
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "agent task `{}` has no completed turn checkpoint",
                    view.task.name
                ))
            });
    }
    if let Ok(index) = selector.parse::<usize>() {
        if index == 0 {
            return Err(Error::InvalidInput(
                "turn index is 1-based; use `crabdb agent turn 1` for the first turn".to_string(),
            ));
        }
        return transcript
            .turns
            .get(index - 1)
            .map(|turn| (index, turn))
            .ok_or_else(|| Error::InvalidInput(format!("turn `{selector}` not found")));
    }
    transcript
        .turns
        .iter()
        .enumerate()
        .find(|(_, turn)| turn.turn.turn_id == selector)
        .map(|(index, turn)| (index + 1, turn))
        .ok_or_else(|| Error::InvalidInput(format!("turn `{selector}` not found")))
}

fn agent_turn_by_selector<'a>(
    view: &'a AgentTaskViewReport,
    selector: &str,
) -> Result<&'a TranscriptTurn> {
    let selector = selector.trim();
    let transcript = view.transcript.as_ref().ok_or_else(|| {
        Error::InvalidInput(format!(
            "agent task `{}` has no turn transcript",
            view.task.name
        ))
    })?;
    let turn = if let Ok(index) = selector.parse::<usize>() {
        if index == 0 {
            return Err(Error::InvalidInput(
                "turn index is 1-based; use turn:1 for the first turn".to_string(),
            ));
        }
        transcript.turns.get(index - 1)
    } else {
        transcript
            .turns
            .iter()
            .find(|turn| turn.turn.turn_id == selector)
    };
    turn.ok_or_else(|| Error::InvalidInput(format!("turn `{selector}` not found")))
}

fn agent_turn_by_prompt<'a>(
    view: &'a AgentTaskViewReport,
    prompt: &str,
) -> Result<&'a TranscriptTurn> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(Error::InvalidInput(
            "prompt rewind target cannot be empty".to_string(),
        ));
    }
    let needle = prompt.to_ascii_lowercase();
    let transcript = view.transcript.as_ref().ok_or_else(|| {
        Error::InvalidInput(format!(
            "agent task `{}` has no turn transcript",
            view.task.name
        ))
    })?;
    transcript
        .turns
        .iter()
        .rev()
        .find(|turn| {
            turn_prompt_body(turn)
                .map(|body| body.to_ascii_lowercase().contains(&needle))
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "no prompt matching `{prompt}` found in agent task `{}`",
                view.task.name
            ))
        })
}

fn agent_turn_checkpoint(turn: &TranscriptTurn) -> Result<ChangeId> {
    turn.checkpoint
        .clone()
        .or_else(|| turn.turn.after_change.clone())
        .ok_or_else(|| {
            Error::InvalidInput(format!("turn `{}` has no checkpoint", turn.turn.turn_id))
        })
}

fn strip_agent_alias_prefix<'a>(value: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let normalized = value.to_ascii_lowercase();
    prefixes
        .iter()
        .find(|prefix| normalized.starts_with(**prefix))
        .map(|prefix| value[prefix.len()..].trim())
}

fn turn_prompt_preview(turn: &TranscriptTurn) -> Option<String> {
    turn_prompt_body(turn).map(|body| single_line_preview(body, 120))
}

fn turn_prompt_body(turn: &TranscriptTurn) -> Option<&str> {
    turn.messages
        .iter()
        .find(|message| message.role == "user")
        .or_else(|| turn.messages.first())
        .map(|message| message.body.as_str())
}

fn turn_assistant_preview(turn: &TranscriptTurn) -> Option<String> {
    turn.messages
        .iter()
        .find(|message| message.role == "assistant")
        .map(|message| single_line_preview(&message.body, 120))
}

fn agent_task_title(
    lane: &str,
    provider: Option<&str>,
    transcript: Option<&TranscriptReport>,
) -> String {
    if let Some(label) = agent_lane_explicit_label(lane, provider) {
        return label;
    }
    if let Some(prompt) = transcript.and_then(agent_transcript_prompt_title) {
        return prompt;
    }
    lane.to_string()
}

fn agent_transcript_prompt_title(transcript: &TranscriptReport) -> Option<String> {
    transcript
        .turns
        .iter()
        .find_map(turn_prompt_body)
        .map(|prompt| single_line_preview(prompt, 80))
        .filter(|prompt| !prompt.trim().is_empty())
}

fn agent_lane_explicit_label(lane: &str, provider: Option<&str>) -> Option<String> {
    let body = lane.strip_prefix("agent-")?;
    let (component, _hash) = body.rsplit_once('-')?;
    if component.is_empty() {
        return None;
    }
    if provider
        .map(sanitize_agent_ref_component)
        .is_some_and(|provider_component| provider_component == component)
    {
        return None;
    }
    Some(
        component
            .chars()
            .map(|ch| {
                if matches!(ch, '-' | '_' | '.') {
                    ' '
                } else {
                    ch
                }
            })
            .collect(),
    )
}

fn single_line_preview(value: &str, limit: usize) -> String {
    let mut preview = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.len() > limit {
        preview.truncate(limit.saturating_sub(3));
        preview.push_str("...");
    }
    preview
}

fn agent_push_suggestion(suggestions: &mut Vec<StatusSuggestion>, command: String, reason: &str) {
    if !suggestions
        .iter()
        .any(|suggestion| suggestion.command == command)
    {
        suggestions.push(StatusSuggestion {
            command,
            reason: reason.to_string(),
        });
    }
}

fn agent_ask_route(question: &str) -> Result<AgentAskRoute> {
    let lowered = question.to_ascii_lowercase();
    let tokens = agent_ask_tokens(question);
    let lowered_tokens = tokens
        .iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let path = agent_ask_path(&tokens, &lowered_tokens);
    let wants_patch = lowered.contains("patch")
        || lowered.contains("diff")
        || lowered.contains("hunk")
        || lowered.contains("unified");
    let wants_turn_diff = wants_patch
        && (lowered.contains("turn")
            || lowered.contains("prompt")
            || lowered.contains("last response")
            || lowered.contains("latest response"));
    let mentions_turn_or_prompt = lowered.contains("turn")
        || lowered.contains("prompt")
        || lowered.contains("last response")
        || lowered.contains("latest response");
    let wants_prompt_change = mentions_turn_or_prompt
        && (lowered.contains("what changed")
            || lowered.contains("changed in")
            || lowered.contains("changed from")
            || lowered.contains("changes in")
            || lowered.contains("changes from")
            || lowered.contains("code changed")
            || agent_ask_has_any(&lowered_tokens, &["changed", "changes", "delta"]));

    if agent_ask_has_any(&lowered_tokens, &["why", "explain", "reason"]) {
        return path.map(AgentAskRoute::Why).ok_or_else(|| {
            Error::InvalidInput(
                "agent ask needs a file path for why/explain questions, for example `crabdb agent ask explain README.md`"
                    .to_string(),
            )
        });
    }
    if wants_turn_diff {
        return Ok(AgentAskRoute::TurnDiff {
            file: path,
            patch: true,
        });
    }
    if let Some(path) = path.clone() {
        if lowered.contains("which prompt")
            || lowered.contains("what prompt")
            || lowered.contains("which turn")
            || lowered.contains("what turn")
            || lowered.contains("which operation")
            || lowered.contains("what operation")
            || lowered.contains("which checkpoint")
            || lowered.contains("what checkpoint")
            || lowered.contains("which tool")
            || lowered.contains("what tool")
            || lowered.contains("who changed")
            || lowered.contains("who touched")
            || lowered.contains("what touched")
            || lowered.contains("what caused")
            || lowered.contains("where did")
            || lowered.contains("came from")
            || agent_ask_has_any(
                &lowered_tokens,
                &["touched", "caused", "introduced", "origin"],
            )
        {
            return Ok(AgentAskRoute::Why(path));
        }
    }
    if wants_prompt_change {
        return Ok(AgentAskRoute::Delta {
            file: path.clone(),
            patch: wants_patch,
        });
    }
    if agent_ask_has_any(&lowered_tokens, &["inspect", "file", "path"]) {
        if let Some(path) = path.clone() {
            return Ok(AgentAskRoute::File {
                path,
                patch: wants_patch,
            });
        }
    }
    if lowered.contains("changed files")
        || lowered.contains("which files")
        || lowered.contains("what files")
        || lowered.contains("edited files")
        || lowered.contains("touched files")
        || lowered.contains("files touched")
        || lowered.contains("where did it edit")
        || lowered.contains("where did the agent edit")
        || lowered.contains("what did it edit")
        || lowered.contains("what did the agent edit")
        || agent_ask_has_any(&lowered_tokens, &["files"])
    {
        return Ok(AgentAskRoute::Files);
    }
    if let Some(path) = path.clone() {
        if lowered.contains("what changed")
            || lowered.contains("changed")
            || lowered.contains("diff")
            || lowered.contains("patch")
        {
            return Ok(AgentAskRoute::File {
                path,
                patch: wants_patch,
            });
        }
    }
    if lowered.contains("what needs attention")
        || lowered.contains("needs my attention")
        || lowered.contains("need my attention")
        || lowered.contains("what is waiting")
        || lowered.contains("what's waiting")
        || lowered.contains("waiting on me")
        || lowered.contains("waiting for me")
        || lowered.contains("show inbox")
        || lowered.contains("agent inbox")
        || lowered.contains("task inbox")
        || lowered.contains("work queue")
        || lowered.contains("task queue")
        || lowered.contains("all agent tasks")
        || lowered.contains("all tasks")
        || matches!(lowered.trim(), "inbox" | "home" | "queue")
    {
        return Ok(AgentAskRoute::Inbox);
    }
    if lowered.contains("workdir")
        || lowered.contains("work dir")
        || lowered.contains("working directory")
        || lowered.contains("task directory")
        || lowered.contains("agent directory")
        || lowered.contains("materialized directory")
        || lowered.contains("materialized checkout")
        || lowered.contains("checkout path")
        || lowered.contains("local checkout")
        || lowered.contains("cd command")
        || lowered.contains("where is the code")
        || lowered.contains("where are the files")
        || lowered.contains("open folder")
        || lowered.contains("open directory")
    {
        return Ok(AgentAskRoute::Workdir);
    }
    if lowered.contains("transcript")
        || lowered.contains("conversation")
        || lowered.contains("message history")
        || lowered.contains("prompt history")
        || agent_ask_has_any(&lowered_tokens, &["chat", "messages"])
    {
        return Ok(AgentAskRoute::View);
    }
    if lowered.contains("turn") || lowered.contains("prompt") {
        return Ok(AgentAskRoute::Turn);
    }
    if lowered.contains("review first")
        || lowered.contains("inspect first")
        || lowered.contains("look at first")
        || lowered.contains("where should i start")
    {
        return Ok(AgentAskRoute::Focus);
    }
    if lowered.contains("review plan")
        || lowered.contains("review dashboard")
        || lowered.contains("review priority")
        || lowered.contains("review priorities")
        || lowered.contains("open review")
        || lowered.contains("start review")
        || lowered.contains("review this task")
        || lowered.contains("review this agent task")
        || lowered.contains("review task")
        || lowered.contains("task review")
        || lowered.contains("what should i review")
        || lowered.contains("what to review")
        || lowered.contains("show review")
    {
        return Ok(AgentAskRoute::Review);
    }
    if lowered.contains("what should")
        || lowered.contains("what now")
        || lowered.contains("next")
        || agent_ask_has_any(&lowered_tokens, &["todo", "help"])
    {
        return Ok(AgentAskRoute::Next);
    }
    if lowered.contains("can land")
        || lowered.contains("can apply")
        || lowered.contains("can merge")
        || lowered.contains("can ship")
        || lowered.contains("safe")
        || lowered.contains("ready")
        || lowered.contains("preflight")
        || agent_ask_has_any(&lowered_tokens, &["land", "apply", "merge", "ship"])
    {
        return Ok(AgentAskRoute::Ready);
    }
    if lowered.contains("recover")
        || lowered.contains("stuck")
        || lowered.contains("sideways")
        || lowered.contains("blocked")
        || lowered.contains("failed")
        || lowered.contains("failure")
    {
        return Ok(AgentAskRoute::Diagnose);
    }
    if lowered.contains("rewind")
        || lowered.contains("checkpoint")
        || lowered.contains("undo")
        || lowered.contains("roll back")
        || lowered.contains("rollback")
    {
        return Ok(AgentAskRoute::Checkpoints);
    }
    if lowered.contains("just changed")
        || lowered.contains("last change")
        || lowered.contains("latest change")
        || lowered.contains("recent change")
        || agent_ask_has_any(&lowered_tokens, &["last"])
    {
        return Ok(AgentAskRoute::Delta {
            file: None,
            patch: wants_patch,
        });
    }
    if lowered.contains("since i looked")
        || lowered.contains("since reviewed")
        || lowered.contains("new changes")
        || lowered.contains("what changed")
    {
        return Ok(AgentAskRoute::New {
            file: None,
            patch: wants_patch,
        });
    }
    if lowered.contains("all changes")
        || lowered.contains("change cards")
        || agent_ask_has_any(&lowered_tokens, &["changes"])
    {
        return Ok(AgentAskRoute::Changes);
    }
    if wants_patch {
        return Ok(AgentAskRoute::Delta {
            file: path,
            patch: true,
        });
    }
    if lowered.contains("timeline") || lowered.contains("chronological") || lowered.contains("when")
    {
        return Ok(AgentAskRoute::Timeline);
    }
    if lowered.contains("turn") || lowered.contains("prompt") {
        return Ok(AgentAskRoute::Turn);
    }
    if lowered.contains("what did")
        || lowered.contains("what happened")
        || lowered.contains("what was done")
        || lowered.contains("what got done")
    {
        return Ok(AgentAskRoute::Story);
    }
    if agent_ask_has_any(&lowered_tokens, &["tools", "tool", "commands", "command"]) {
        return Ok(AgentAskRoute::View);
    }
    if lowered.contains("test status")
        || lowered.contains("validation")
        || lowered.contains("what tests")
        || lowered.contains("which tests")
        || lowered.contains("do i need tests")
        || lowered.contains("need tests")
    {
        return Ok(AgentAskRoute::Validate);
    }
    if lowered.contains("summary") || lowered.contains("overview") || lowered.contains("cockpit") {
        return Ok(AgentAskRoute::Summary);
    }
    if lowered.contains("brief") {
        return Ok(AgentAskRoute::Brief);
    }
    if lowered.contains("story") || lowered.contains("happened") {
        return Ok(AgentAskRoute::Story);
    }
    if lowered.contains("risk") {
        return Ok(AgentAskRoute::Risk);
    }
    if lowered.contains("receipt") {
        return Ok(AgentAskRoute::Receipt);
    }
    if lowered.contains("pr") || lowered.contains("pull request") {
        return Ok(AgentAskRoute::Pr);
    }
    if lowered.contains("review") {
        return Ok(AgentAskRoute::Review);
    }
    if lowered.contains("focus") || lowered.contains("first file") {
        return Ok(AgentAskRoute::Focus);
    }
    if lowered.contains("view") || lowered.contains("transcript") {
        return Ok(AgentAskRoute::View);
    }

    Err(Error::InvalidInput(format!(
        "could not route agent question `{question}`; try `what should I do next`, `what changed`, `what just changed`, `is it safe to land`, `recover`, `changed files`, or `explain README.md`"
    )))
}

fn agent_ask_tokens(question: &str) -> Vec<String> {
    question
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    matches!(
                        ch,
                        '"' | '\''
                            | '`'
                            | ','
                            | ';'
                            | ':'
                            | '?'
                            | '!'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                    )
                })
                .to_string()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

fn agent_ask_has_any(tokens: &[String], words: &[&str]) -> bool {
    tokens
        .iter()
        .any(|token| words.iter().any(|word| token == word))
}

fn agent_ask_path(tokens: &[String], lowered_tokens: &[String]) -> Option<String> {
    for (idx, token) in lowered_tokens.iter().enumerate() {
        if matches!(
            token.as_str(),
            "why" | "explain" | "inspect" | "file" | "path"
        ) {
            if let Some(path) = tokens
                .get(idx + 1)
                .and_then(|value| agent_ask_clean_path(value))
            {
                return Some(path);
            }
        }
    }
    tokens.iter().find_map(|token| agent_ask_clean_path(token))
}

fn agent_ask_clean_path(token: &str) -> Option<String> {
    let value = token.trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\'' | '`' | ',' | ';' | ':' | '?' | '!' | '(' | ')' | '[' | ']' | '{' | '}'
        )
    });
    if value.is_empty() || value.starts_with("--") {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "what"
            | "changed"
            | "change"
            | "changes"
            | "why"
            | "explain"
            | "inspect"
            | "file"
            | "path"
            | "in"
            | "for"
            | "the"
            | "latest"
            | "task"
    ) {
        return None;
    }
    if value.contains('/') || value.contains('\\') || value.contains('.') {
        return Some(value.to_string());
    }
    None
}

fn agent_inbox_groups(items: &[AgentInboxItem]) -> Vec<AgentInboxGroup> {
    [
        AgentTaskStatus::Dirty,
        AgentTaskStatus::Conflicted,
        AgentTaskStatus::Blocked,
        AgentTaskStatus::Ready,
        AgentTaskStatus::Active,
        AgentTaskStatus::Applied,
        AgentTaskStatus::Empty,
    ]
    .into_iter()
    .filter_map(|status| {
        let group_items = items
            .iter()
            .filter(|item| item.task.status == status)
            .cloned()
            .collect::<Vec<_>>();
        let group_tasks = group_items
            .iter()
            .map(|item| item.task.clone())
            .collect::<Vec<_>>();
        if group_tasks.is_empty() {
            return None;
        }
        let next = group_items.first().map(|item| item.next.clone());
        Some(AgentInboxGroup {
            key: agent_inbox_status_key(&status).to_string(),
            label: agent_inbox_status_label(&status).to_string(),
            status,
            tasks: group_tasks,
            items: group_items,
            next,
        })
    })
    .collect()
}

fn agent_inbox_next_for_task(task: &AgentTaskReport) -> StatusSuggestion {
    let lane = &task.lane;
    match &task.status {
        AgentTaskStatus::Empty => StatusSuggestion {
            command: "crabdb agent setup --provider claude-code --editor vscode".to_string(),
            reason: "configure an editor once, then start an agent task".to_string(),
        },
        AgentTaskStatus::Active => StatusSuggestion {
            command: format!("crabdb agent view {lane}"),
            reason: "inspect current task activity, transcript, and captured events".to_string(),
        },
        AgentTaskStatus::Dirty => StatusSuggestion {
            command: format!("crabdb agent land {lane} --dry-run"),
            reason: "preview recording the workdir and applying the task without mutating Git"
                .to_string(),
        },
        AgentTaskStatus::Ready => StatusSuggestion {
            command: format!("crabdb agent review-plan {lane}"),
            reason: "review changed files, transcript, blockers, warnings, and next steps"
                .to_string(),
        },
        AgentTaskStatus::Blocked => StatusSuggestion {
            command: format!("crabdb agent review-plan {lane}"),
            reason: "inspect blockers and decide whether to continue, rewind, or discard"
                .to_string(),
        },
        AgentTaskStatus::Conflicted => StatusSuggestion {
            command: format!("crabdb agent review-plan {lane}"),
            reason: "inspect conflicts before any apply attempt".to_string(),
        },
        AgentTaskStatus::Applied => StatusSuggestion {
            command: format!("crabdb agent view {lane}"),
            reason: "inspect the applied transcript, tools, and checkpoint".to_string(),
        },
    }
}

fn agent_inbox_status_key(status: &AgentTaskStatus) -> &'static str {
    match status {
        AgentTaskStatus::Dirty => "needs_record",
        AgentTaskStatus::Conflicted => "conflicted",
        AgentTaskStatus::Blocked => "blocked",
        AgentTaskStatus::Ready => "ready",
        AgentTaskStatus::Active => "active",
        AgentTaskStatus::Applied => "applied",
        AgentTaskStatus::Empty => "empty",
    }
}

fn agent_inbox_status_label(status: &AgentTaskStatus) -> &'static str {
    match status {
        AgentTaskStatus::Dirty => "Needs record or apply",
        AgentTaskStatus::Conflicted => "Conflicted",
        AgentTaskStatus::Blocked => "Blocked",
        AgentTaskStatus::Ready => "Ready to apply",
        AgentTaskStatus::Active => "Active",
        AgentTaskStatus::Applied => "Applied",
        AgentTaskStatus::Empty => "Empty",
    }
}

fn filter_agent_groups_by_path(groups: Vec<AgentChangeGroup>, path: &str) -> Vec<AgentChangeGroup> {
    groups
        .into_iter()
        .filter_map(|mut group| {
            group
                .changed_paths
                .retain(|file| agent_file_matches_path(file, path));
            (!group.changed_paths.is_empty()).then_some(group)
        })
        .collect()
}

fn agent_file_matches_path(file: &FileDiffSummary, path: &str) -> bool {
    file.path == path || file.old_path.as_deref() == Some(path)
}

fn agent_filter_diff_to_path(mut diff: DiffSummary, path: &str) -> Result<DiffSummary> {
    let original_count = diff.files.len();
    diff.files
        .retain(|file| agent_file_matches_path(file, path));
    if diff.files.is_empty() && original_count > 0 {
        return Err(Error::InvalidInput(format!(
            "`{path}` is not changed in the selected agent diff target"
        )));
    }
    Ok(diff)
}

fn strip_agent_path_line_suffix(value: &str) -> &str {
    let Some((path, line)) = value.rsplit_once(':') else {
        return value;
    };
    if !path.is_empty() && line.parse::<u64>().is_ok() {
        path
    } else {
        value
    }
}

fn agent_why_summary(task: &AgentTaskReport, path: &str, groups: &[AgentChangeGroup]) -> String {
    if groups.is_empty() {
        return format!(
            "`{path}` is not recorded as changed in agent task `{}`.",
            task.title
        );
    }
    let unit = if groups.iter().all(|group| group.kind == "turn") {
        if groups.len() == 1 {
            "turn"
        } else {
            "turns"
        }
    } else if groups.len() == 1 {
        "operation"
    } else {
        "operations"
    };
    let first_prompt = groups
        .first()
        .and_then(|group| group.prompt_preview.as_deref())
        .filter(|prompt| !prompt.trim().is_empty());
    if let Some(prompt) = first_prompt {
        format!(
            "`{path}` changed in {} {unit} for agent task `{}`. First related prompt: {prompt}",
            groups.len(),
            task.title
        )
    } else {
        format!(
            "`{path}` changed in {} {unit} for agent task `{}`.",
            groups.len(),
            task.title
        )
    }
}

fn agent_story_summary(view: &AgentTaskViewReport, groups: &[AgentChangeGroup]) -> String {
    let title = if view.task.title.trim().is_empty() {
        &view.task.name
    } else {
        &view.task.title
    };
    let status = agent_story_status_phrase(&view.task.status);
    let changed = agent_story_changed_phrase(&view.task.changed_paths);
    let turn_count = view
        .transcript
        .as_ref()
        .map(|transcript| transcript.turns.len())
        .unwrap_or(groups.len());
    let first_prompt = groups
        .iter()
        .find_map(|group| group.prompt_preview.as_deref())
        .map(|prompt| single_line_preview(prompt, 96));

    if groups.is_empty() && view.task.changed_paths.is_empty() {
        return format!(
            "No code checkpoint has been recorded for {title} yet; the task is {status}."
        );
    }
    if let Some(prompt) = first_prompt {
        return format!("The agent worked on \"{prompt}\", changed {changed}, and is {status}.");
    }
    let turn_word = if turn_count == 1 { "turn" } else { "turns" };
    format!("The agent recorded {turn_count} {turn_word}, changed {changed}, and is {status}.")
}

fn agent_story_status_phrase(status: &AgentTaskStatus) -> &'static str {
    match status {
        AgentTaskStatus::Empty => "empty",
        AgentTaskStatus::Active => "still active or waiting for a checkpoint",
        AgentTaskStatus::Dirty => "waiting for the workdir changes to be recorded",
        AgentTaskStatus::Ready => "ready for review",
        AgentTaskStatus::Blocked => "blocked",
        AgentTaskStatus::Conflicted => "conflicted",
        AgentTaskStatus::Applied => "already applied",
    }
}

fn agent_story_changed_phrase(paths: &[FileDiffSummary]) -> String {
    match paths {
        [] => "no files".to_string(),
        [path] => format!("`{}`", path.path),
        [first, rest @ ..] => {
            let file_word = if rest.len() == 1 { "file" } else { "files" };
            format!("`{}` and {} other {file_word}", first.path, rest.len())
        }
    }
}

fn agent_story_risk_notes(view: &AgentTaskViewReport) -> Vec<String> {
    let mut notes = Vec::new();
    for blocker in &view.review.readiness.blockers {
        notes.push(format!("Blocker: {} - {}", blocker.code, blocker.message));
    }
    for warning in &view.review.readiness.warnings {
        notes.push(format!("Warning: {} - {}", warning.code, warning.message));
    }
    if view.task.status == AgentTaskStatus::Dirty
        && !view
            .review
            .readiness
            .blockers
            .iter()
            .any(|blocker| blocker.code == "dirty_workdir")
    {
        notes.push("The materialized task workdir has unrecorded changes.".to_string());
    }
    if view.task.latest_checkpoint.is_none() {
        notes.push("No checkpoint has been recorded for this task yet.".to_string());
    }
    notes
}

fn agent_risk_report_from_view(view: &AgentTaskViewReport) -> AgentRiskReport {
    let mut score: u8 = 0;
    let mut reasons = Vec::new();
    let mut recommendations = Vec::new();
    let lane = &view.task.lane;

    for blocker in &view.review.readiness.blockers {
        score = score.saturating_add(60);
        agent_push_risk_reason(&mut reasons, &blocker.code, "blocking", &blocker.message);
    }
    for warning in &view.review.readiness.warnings {
        let weight = agent_warning_risk_weight(&warning.code);
        score = score.saturating_add(weight);
        agent_push_risk_reason(&mut reasons, &warning.code, "medium", &warning.message);
    }

    match &view.task.status {
        AgentTaskStatus::Conflicted | AgentTaskStatus::Blocked => {
            score = score.saturating_add(70);
            agent_push_risk_reason(
                &mut reasons,
                "task_not_ready",
                "blocking",
                "the task has blockers or conflicts and cannot be applied safely yet",
            );
        }
        AgentTaskStatus::Dirty => {
            score = score.saturating_add(45);
            agent_push_risk_reason(
                &mut reasons,
                "dirty_workdir",
                "high",
                "the materialized task workdir has unrecorded changes",
            );
        }
        AgentTaskStatus::Active => {
            score = score.saturating_add(25);
            agent_push_risk_reason(
                &mut reasons,
                "active_task",
                "medium",
                "the task is still active or has no final checkpoint yet",
            );
        }
        AgentTaskStatus::Empty | AgentTaskStatus::Ready | AgentTaskStatus::Applied => {}
    }

    let changed_count = view.task.changed_paths.len();
    if changed_count == 0 {
        agent_push_risk_reason(
            &mut reasons,
            "no_changed_files",
            "low",
            "no changed files are recorded for this task",
        );
    } else if changed_count >= 15 {
        score = score.saturating_add(25);
        agent_push_risk_reason(
            &mut reasons,
            "large_change_set",
            "high",
            &format!("{changed_count} files changed"),
        );
    } else if changed_count >= 5 {
        score = score.saturating_add(10);
        agent_push_risk_reason(
            &mut reasons,
            "medium_change_set",
            "medium",
            &format!("{changed_count} files changed"),
        );
    }

    if view.task.latest_checkpoint.is_none() {
        score = score.saturating_add(20);
        agent_push_risk_reason(
            &mut reasons,
            "missing_checkpoint",
            "medium",
            "no checkpoint has been recorded for this task",
        );
    }

    if agent_changed_manifest_or_api_surface(&view.task.changed_paths) {
        score = score.saturating_add(15);
        agent_push_risk_reason(
            &mut reasons,
            "public_surface_or_manifest_changed",
            "medium",
            "the task touched package manifests, lockfiles, or public API-looking files",
        );
    }

    let level = if reasons.iter().any(|reason| reason.severity == "blocking") {
        AgentRiskLevel::Blocking
    } else if score >= 70 {
        AgentRiskLevel::High
    } else if score >= 35 {
        AgentRiskLevel::Medium
    } else {
        AgentRiskLevel::Low
    };
    let score = score.min(100);
    let summary = agent_risk_summary(&level, score, &view.task);

    if matches!(&level, AgentRiskLevel::Blocking | AgentRiskLevel::High) {
        agent_push_suggestion(
            &mut recommendations,
            format!("crabdb agent review-plan {lane}"),
            "inspect blockers, conflicts, warnings, and changed files before applying",
        );
    }
    if reasons
        .iter()
        .any(|reason| reason.code == "missing_latest_test")
    {
        agent_push_suggestion(
            &mut recommendations,
            format!("crabdb agent validate {lane}"),
            "see suggested validation commands before applying",
        );
    }
    if reasons.iter().any(|reason| {
        matches!(
            reason.code.as_str(),
            "large_change_set" | "medium_change_set"
        )
    }) {
        agent_push_suggestion(
            &mut recommendations,
            format!("crabdb agent changes {lane}"),
            "review changes grouped by prompt or operation",
        );
    }
    if view.task.workdir.is_some()
        && reasons.iter().any(|reason| {
            matches!(
                reason.code.as_str(),
                "dirty_workdir" | "public_surface_or_manifest_changed"
            )
        })
    {
        agent_push_suggestion(
            &mut recommendations,
            format!("crabdb agent workdir {lane}"),
            "open the materialized task workdir for local inspection",
        );
    }
    agent_push_suggestion(
        &mut recommendations,
        format!("crabdb agent land {lane} --dry-run"),
        "preview whether the task can be applied safely",
    );

    AgentRiskReport {
        task: view.task.clone(),
        level,
        score,
        summary,
        reasons,
        recommendations,
    }
}

fn agent_ready_status(
    view: &AgentTaskViewReport,
    apply_preview: Option<&AgentApplyReport>,
    apply_error: Option<&str>,
) -> String {
    if view.task.status == AgentTaskStatus::Applied {
        return "applied".to_string();
    }
    if !view.review.readiness.ready {
        return view.review.readiness.status.clone();
    }
    if apply_error.is_some() {
        return "git_blocked".to_string();
    }
    match apply_preview.map(|report| report.status.as_str()) {
        Some("ready") => "ready".to_string(),
        Some("would_record") => "needs_record".to_string(),
        Some("conflicted") => "conflicted".to_string(),
        Some(status) => status.to_string(),
        None => "unknown".to_string(),
    }
}

fn agent_ready_summary(
    view: &AgentTaskViewReport,
    risk: &AgentRiskReport,
    ready: bool,
    status: &str,
    apply_error: Option<&str>,
) -> String {
    let title = agent_task_label(&view.task);
    if ready {
        return format!(
            "`{title}` is ready to apply: {} changed file(s), risk {:?} ({}/100), and Git dry-run preflight passed.",
            view.task.changed_paths.len(),
            risk.level,
            risk.score
        );
    }
    if let Some(error) = apply_error {
        return format!("`{title}` is not ready to apply because Git preflight failed: {error}");
    }
    match status {
        "needs_record" => format!(
            "`{title}` has unrecorded task workdir changes; record or apply once the checkpoint is intentional."
        ),
        "conflicted" => {
            format!("`{title}` is not ready to apply because the dry-run merge found conflicts.")
        }
        "applied" => format!("`{title}` has already been applied."),
        _ => format!(
            "`{title}` is not ready to apply: readiness status `{status}`, risk {:?} ({}/100).",
            risk.level, risk.score
        ),
    }
}

fn agent_ready_next(
    lane: &str,
    crab_branch: &str,
    status: &str,
    apply_preview: Option<&AgentApplyReport>,
    apply_error: Option<&str>,
) -> StatusSuggestion {
    if status == "ready" {
        return apply_preview
            .and_then(|report| report.suggestions.first().cloned())
            .unwrap_or_else(|| StatusSuggestion {
                command: format!("crabdb agent land {lane}"),
                reason: "apply the task after the clean readiness preflight".to_string(),
            });
    }
    if status == "needs_record" {
        return StatusSuggestion {
            command: format!("crabdb agent land {lane}"),
            reason: "record the task workdir first, then apply if the resulting merge is clean"
                .to_string(),
        };
    }
    if status == "conflicted" {
        return StatusSuggestion {
            command: format!("crabdb agent review-plan {lane}"),
            reason: "inspect conflicts and changed files before applying".to_string(),
        };
    }
    if status == "applied" {
        return StatusSuggestion {
            command: format!("crabdb agent view {lane}"),
            reason: "inspect the applied task transcript and checkpoint".to_string(),
        };
    }
    if let Some(error) = apply_error {
        if error.contains("current Git worktree has tracked changes") {
            return StatusSuggestion {
                command: "git status --short".to_string(),
                reason: "inspect tracked Git changes before running CrabDB apply".to_string(),
            };
        }
        if error.contains("does not match CrabDB branch") {
            return StatusSuggestion {
                command: format!("crabdb git import-update --branch {crab_branch}"),
                reason: "refresh CrabDB's internal base or switch to a matching Git branch"
                    .to_string(),
            };
        }
    }
    StatusSuggestion {
        command: format!("crabdb agent review-plan {lane}"),
        reason: "inspect blockers, warnings, changed files, and provenance before applying"
            .to_string(),
    }
}

fn agent_ready_suggestions(
    view: &AgentTaskViewReport,
    risk: &AgentRiskReport,
    primary: &StatusSuggestion,
    apply_preview: Option<&AgentApplyReport>,
) -> Vec<StatusSuggestion> {
    let lane = &view.task.lane;
    let mut suggestions = vec![primary.clone()];
    if let Some(preview) = apply_preview {
        for suggestion in &preview.suggestions {
            agent_push_suggestion(
                &mut suggestions,
                suggestion.command.clone(),
                &suggestion.reason,
            );
        }
    }
    for suggestion in &risk.recommendations {
        agent_push_suggestion(
            &mut suggestions,
            suggestion.command.clone(),
            &suggestion.reason,
        );
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane}"),
        "review high-level change cards before applying",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent turn {lane}"),
        "inspect the latest completed prompt-sized turn",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "see the raw apply dry-run plan",
    );
    suggestions
}

fn agent_compare_path_map(paths: &[FileDiffSummary]) -> BTreeMap<String, FileDiffSummary> {
    paths
        .iter()
        .map(|path| (path.path.clone(), path.clone()))
        .collect()
}

fn agent_report_summary(
    view: &AgentTaskViewReport,
    risk: &AgentRiskReport,
    changes: &AgentChangesReport,
) -> String {
    let title = agent_task_label(&view.task);
    let changed_count = view.task.changed_paths.len();
    let turn_count = view
        .transcript
        .as_ref()
        .map(|transcript| transcript.turns.len())
        .unwrap_or(changes.groups.len());
    format!(
        "{title}: {:?}, {changed_count} changed file(s), {turn_count} turn(s), {:?} risk ({}/100).",
        view.task.status, risk.level, risk.score
    )
}

fn agent_report_suggestions(
    view: &AgentTaskViewReport,
    primary: &StatusSuggestion,
) -> Vec<StatusSuggestion> {
    let lane = &view.task.lane;
    let mut suggestions = vec![primary.clone()];
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane}"),
        "review changes grouped by prompt or operation",
    );
    agent_push_suggestion(
        &mut suggestions,
        agent_turn_diff_command(lane, None, None, true),
        "inspect the most recent turn patch",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent validate {lane}"),
        "see suggested validation commands before applying",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview applying the task safely",
    );
    suggestions
}

fn agent_receipt_validation(review: &LaneReviewPacketReport) -> Vec<LaneTestSummary> {
    let mut validation = Vec::new();
    if let Some(test) = &review.latest_test {
        validation.push(test.clone());
    }
    if let Some(eval) = &review.latest_eval {
        if !validation
            .iter()
            .any(|gate: &LaneTestSummary| gate.event_id == eval.event_id)
        {
            validation.push(eval.clone());
        }
    }
    for gate in &review.recent_gates {
        if validation
            .iter()
            .any(|existing: &LaneTestSummary| existing.event_id == gate.event_id)
        {
            continue;
        }
        validation.push(gate.clone());
        if validation.len() >= 5 {
            break;
        }
    }
    validation
}

fn agent_summary_text(receipt: &AgentReceiptReport, ready: &AgentReadyReport) -> String {
    let title = agent_task_label(&receipt.task);
    let readiness = if ready.ready {
        "ready to apply".to_string()
    } else if let Some(error) = &ready.apply_error {
        format!("not ready: {error}")
    } else {
        format!("not ready: {}", ready.status)
    };
    format!(
        "{title}: {:?}, {} changed file(s), {} turn(s), {:?} risk ({}/100), {readiness}.",
        receipt.status,
        receipt.changed_paths.len(),
        receipt.turns.len(),
        ready.risk.level,
        ready.risk.score
    )
}

fn agent_summary_suggestions(
    lane: &str,
    next: &StatusSuggestion,
    ready: bool,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent receipt {lane}"),
        "print the copyable post-run receipt",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent pr {lane}"),
        "print a pull request title and body draft",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane}"),
        "review high-level change cards",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent focus {lane}"),
        "inspect the next highest-priority file",
    );
    if ready {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent land {lane} --dry-run"),
            "preview applying this task before mutating Git",
        );
    }
    suggestions
}

fn agent_validation_needs_gate(latest: &Option<LaneTestSummary>, _missing_code: &str) -> bool {
    match latest {
        Some(gate) => !gate.success,
        None => true,
    }
}

fn agent_validation_status(
    latest_test: &Option<LaneTestSummary>,
    latest_eval: &Option<LaneTestSummary>,
    needs_test: bool,
    needs_eval: bool,
) -> String {
    if latest_test.as_ref().is_some_and(|gate| !gate.success) {
        "test_failed".to_string()
    } else if needs_test {
        "missing_test".to_string()
    } else if latest_eval.as_ref().is_some_and(|gate| !gate.success) {
        "eval_failed".to_string()
    } else if needs_eval {
        "missing_eval".to_string()
    } else {
        "ok".to_string()
    }
}

fn agent_validation_summary(
    task: &AgentTaskReport,
    status: &str,
    needs_test: bool,
    needs_eval: bool,
) -> String {
    let title = agent_task_label(task);
    match status {
        "test_failed" => format!(
            "{title}: latest test gate failed; rerun or inspect validation before applying."
        ),
        "missing_test" => format!(
            "{title}: no passing test gate is recorded for {} changed file(s).",
            task.changed_paths.len()
        ),
        "eval_failed" => {
            format!("{title}: latest eval gate failed; inspect quality validation before applying.")
        }
        "missing_eval" => format!(
            "{title}: no eval gate is recorded; run one when model or policy quality matters."
        ),
        _ if needs_test || needs_eval => {
            format!("{title}: validation still needs attention before applying.")
        }
        _ => format!("{title}: latest recorded validation gates are passing."),
    }
}

fn agent_validation_suggestions(
    lane: &str,
    workspace_root: &Path,
    changed_paths: &[FileDiffSummary],
    needs_test: bool,
    needs_eval: bool,
) -> Vec<StatusSuggestion> {
    let mut suggestions = Vec::new();
    if needs_test {
        let test_command = agent_validation_default_test_command(workspace_root, false);
        agent_push_suggestion(
            &mut suggestions,
            format!(
                "crabdb agent test {lane} -- {}",
                agent_validation_command_text(&test_command)
            ),
            "record a passing test gate for this task",
        );
    }
    if agent_changed_manifest_or_api_surface(changed_paths) {
        let broad_command = agent_validation_default_test_command(workspace_root, true);
        agent_push_suggestion(
            &mut suggestions,
            format!(
                "crabdb agent test {lane} -- {}",
                agent_validation_command_text(&broad_command)
            ),
            "run a broader test gate because manifests, lockfiles, or public API-looking files changed",
        );
    }
    if needs_eval {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent eval {lane} -- <eval-command>"),
            "record an eval gate when model or policy quality matters",
        );
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent ready {lane}"),
        "check apply readiness after validation",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview apply after validation passes",
    );
    suggestions
}

fn agent_validation_default_test_command(workspace_root: &Path, broad: bool) -> Vec<String> {
    if workspace_root.join("Cargo.toml").exists() {
        if broad {
            return vec![
                "cargo".to_string(),
                "test".to_string(),
                "--all".to_string(),
                "--all-targets".to_string(),
            ];
        }
        return vec!["cargo".to_string(), "test".to_string()];
    }
    if workspace_root.join("package.json").exists() {
        return vec!["npm".to_string(), "test".to_string()];
    }
    vec!["<test-command>".to_string()]
}

fn agent_validation_command_text(command: &[String]) -> String {
    if command.len() == 1 && command[0].starts_with('<') && command[0].ends_with('>') {
        return command[0].clone();
    }
    command
        .iter()
        .map(|part| agent_shell_arg(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn agent_diagnosis_assessment(
    ready: &AgentReadyReport,
    checkpoints: &AgentCheckpointReport,
) -> (String, String, String, Vec<String>) {
    let mut evidence = Vec::new();
    for blocker in &ready.blockers {
        evidence.push(format!("blocker `{}`: {}", blocker.code, blocker.message));
    }
    for warning in ready.warnings.iter().take(3) {
        evidence.push(format!("warning `{}`: {}", warning.code, warning.message));
    }
    for reason in ready.risk.reasons.iter().take(3) {
        evidence.push(format!(
            "risk `{}` [{}]: {}",
            reason.code, reason.severity, reason.message
        ));
    }
    if let Some(error) = &ready.apply_error {
        evidence.push(format!("Git preflight failed: {error}"));
    }
    if let Some(checkpoint) = &ready.task.latest_checkpoint {
        evidence.push(format!("latest checkpoint `{}`", checkpoint.0));
    }
    evidence.push(format!(
        "{} friendly checkpoint target(s) available",
        checkpoints.entries.len()
    ));

    if let Some(blocker) = ready.blockers.first() {
        return (
            "blocked".to_string(),
            "high".to_string(),
            format!("{}: {}", blocker.code, blocker.message),
            evidence,
        );
    }
    if let Some(error) = &ready.apply_error {
        return (
            "git_blocked".to_string(),
            "high".to_string(),
            error.clone(),
            evidence,
        );
    }
    if matches!(
        ready.risk.level,
        AgentRiskLevel::Blocking | AgentRiskLevel::High
    ) {
        return (
            "risky".to_string(),
            "high".to_string(),
            ready.risk.summary.clone(),
            evidence,
        );
    }
    if let Some(warning) = ready
        .warnings
        .iter()
        .find(|warning| warning.code == "missing_latest_test")
        .or_else(|| ready.warnings.first())
    {
        return (
            "needs_validation".to_string(),
            "medium".to_string(),
            format!("{}: {}", warning.code, warning.message),
            evidence,
        );
    }
    if matches!(
        ready.task.status,
        AgentTaskStatus::Dirty | AgentTaskStatus::Blocked | AgentTaskStatus::Conflicted
    ) {
        return (
            "needs_attention".to_string(),
            "medium".to_string(),
            format!("task status is {:?}", ready.task.status),
            evidence,
        );
    }
    if ready.ready {
        return (
            "ok".to_string(),
            "low".to_string(),
            "no blocking issue detected".to_string(),
            evidence,
        );
    }
    (
        "needs_review".to_string(),
        "medium".to_string(),
        ready.summary.clone(),
        evidence,
    )
}

fn agent_diagnosis_recovery_options(
    lane: &str,
    ready: &AgentReadyReport,
    checkpoints: &AgentCheckpointReport,
) -> Vec<StatusSuggestion> {
    let mut options = Vec::new();
    agent_push_suggestion(
        &mut options,
        agent_turn_diff_command(lane, None, None, true),
        "inspect the latest completed turn before changing task state",
    );
    if !checkpoints.entries.is_empty() {
        agent_push_suggestion(
            &mut options,
            format!("crabdb agent undo {lane}"),
            "destructive: undo the latest completed turn if it went sideways",
        );
        if let Some(command) = checkpoints
            .entries
            .iter()
            .rev()
            .find_map(|entry| entry.rewind_before_command.clone())
        {
            agent_push_suggestion(
                &mut options,
                command,
                "destructive: rewind to the state before the latest checkpoint target",
            );
        }
        agent_push_suggestion(
            &mut options,
            format!("crabdb agent checkpoints {lane}"),
            "list all friendly rewind targets and exact checkpoint ids",
        );
    }
    if !ready.ready {
        agent_push_suggestion(&mut options, ready.next.command.clone(), &ready.next.reason);
    }
    agent_push_suggestion(
        &mut options,
        format!("crabdb agent summary {lane}"),
        "return to the one-page task cockpit after recovery",
    );
    options
}

fn agent_diagnosis_suggestions(
    lane: &str,
    next: &StatusSuggestion,
    recovery_options: &[StatusSuggestion],
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    for option in recovery_options.iter().skip(1).take(4) {
        agent_push_suggestion(&mut suggestions, option.command.clone(), &option.reason);
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent focus {lane} --patch"),
        "inspect the highest-priority changed file with a focused patch",
    );
    suggestions
}

fn agent_diagnosis_summary(ready: &AgentReadyReport, status: &str, likely_issue: &str) -> String {
    let title = agent_task_label(&ready.task);
    format!(
        "{title}: diagnosis `{status}`, ready {}, {:?} risk ({}/100). Likely issue: {likely_issue}.",
        ready.ready, ready.risk.level, ready.risk.score
    )
}

fn agent_review_summary(
    view: &AgentTaskViewReport,
    risk: &AgentRiskReport,
    priorities: &[AgentReviewPriority],
) -> String {
    let title = agent_task_label(&view.task);
    let priority_count = priorities.len();
    let file_word = if priority_count == 1 { "file" } else { "files" };
    format!(
        "{title}: {:?}, {} changed file(s), {} turn(s), {:?} risk ({}/100). Review {priority_count} prioritized {file_word} before apply.",
        view.task.status,
        view.task.changed_paths.len(),
        view.task.turns,
        risk.level,
        risk.score
    )
}

fn agent_review_next(
    view: &AgentTaskViewReport,
    risk: &AgentRiskReport,
    priorities: &[AgentReviewPriority],
) -> StatusSuggestion {
    let lane = &view.task.lane;
    if let Some(blocker) = view.review.readiness.blockers.first() {
        return StatusSuggestion {
            command: format!("crabdb agent changes {lane}"),
            reason: format!(
                "resolve blocker `{}` before applying: {}",
                blocker.code, blocker.message
            ),
        };
    }
    if view
        .review
        .readiness
        .warnings
        .iter()
        .any(|warning| warning.code == "missing_latest_test")
    {
        return StatusSuggestion {
            command: format!("crabdb agent validate {lane}"),
            reason: "check suggested validation commands before applying this agent task"
                .to_string(),
        };
    }
    if matches!(
        risk.level,
        AgentRiskLevel::Blocking | AgentRiskLevel::High | AgentRiskLevel::Medium
    ) {
        if let Some(priority) = priorities.first() {
            return StatusSuggestion {
                command: priority.why_command.clone(),
                reason: "start review with the highest-priority changed file".to_string(),
            };
        }
    }
    if view.task.status == AgentTaskStatus::Ready {
        return StatusSuggestion {
            command: format!("crabdb agent land {lane} --dry-run"),
            reason: "preview the safe Git apply plan".to_string(),
        };
    }
    agent_next_report_from_view(view, None).primary
}

fn agent_review_suggestions(
    view: &AgentTaskViewReport,
    primary: &StatusSuggestion,
    priorities: &[AgentReviewPriority],
) -> Vec<StatusSuggestion> {
    let lane = &view.task.lane;
    let mut suggestions = vec![primary.clone()];
    if let Some(priority) = priorities.first() {
        agent_push_suggestion(
            &mut suggestions,
            priority.why_command.clone(),
            "explain why the highest-priority file changed",
        );
        if let Some(diff_command) = &priority.diff_command {
            agent_push_suggestion(
                &mut suggestions,
                diff_command.clone(),
                "inspect the patch behind the highest-priority file",
            );
        }
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane}"),
        "see prompt-to-checkpoint changes grouped by turn or operation",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent report {lane} --markdown"),
        "print a copyable review and handoff report",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview applying this task without mutating Git",
    );
    suggestions
}

fn agent_focus_summary(
    review: &AgentReviewReport,
    path: &str,
    source: &str,
    priority: Option<&AgentReviewPriority>,
    why: &AgentWhyReport,
) -> String {
    let title = agent_task_label(&review.task);
    let source_phrase = if source == "review_priority" {
        "highest-priority changed file"
    } else {
        "selected changed file"
    };
    let reason = priority
        .and_then(|priority| priority.reasons.first())
        .map(|reason| format!(" Review reason: {reason}."))
        .unwrap_or_default();
    format!(
        "`{path}` is the {source_phrase} for `{title}`. {}{}",
        why.summary, reason
    )
}

fn agent_focus_suggestions(
    lane: &str,
    path: &str,
    primary: &StatusSuggestion,
    review: &AgentReviewReport,
) -> Vec<StatusSuggestion> {
    let file_arg = agent_shell_arg(path);
    let mut suggestions = vec![primary.clone()];
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent why {lane} {file_arg}"),
        "show the prompt, turn, tools, and checkpoint for this file",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent review-plan {lane}"),
        "return to the full review dashboard",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent validate {lane}"),
        "check suggested validation commands before applying",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview applying this task safely",
    );
    for suggestion in &review.suggestions {
        agent_push_suggestion(
            &mut suggestions,
            suggestion.command.clone(),
            &suggestion.reason,
        );
    }
    suggestions
}

fn agent_focus_diff_target(groups: &[AgentChangeGroup]) -> (Option<String>, Option<String>) {
    let Some(group) = groups.first() else {
        return (None, None);
    };
    if group.kind == "turn" {
        return (Some(group.index.to_string()), None);
    }
    if let Some(operation) = &group.operation_id {
        return (None, Some(operation.0.clone()));
    }
    (None, None)
}

fn agent_turn_suggestions(
    lane: &str,
    index: usize,
    file: Option<&str>,
    patches: bool,
    changed_paths: &[FileDiffSummary],
) -> Vec<StatusSuggestion> {
    let mut suggestions = Vec::new();
    if !patches {
        let command = if let Some(file) = file {
            format!(
                "crabdb agent turn {lane} {index} --file {} --patch",
                agent_shell_arg(file)
            )
        } else {
            format!("crabdb agent turn {lane} {index} --patch")
        };
        agent_push_suggestion(
            &mut suggestions,
            command,
            "show this turn with unified patch text",
        );
    }
    if let Some(path) = changed_paths.first().map(|path| path.path.as_str()) {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent why {lane} {}", agent_shell_arg(path)),
            "explain the first changed file in this turn",
        );
        agent_push_suggestion(
            &mut suggestions,
            agent_turn_diff_command(lane, Some(index), Some(path), true),
            "inspect the first changed file patch from this turn",
        );
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane}"),
        "return to the task change-card overview",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview applying the whole agent task",
    );
    suggestions
}

fn agent_change_cards(
    lane: &str,
    paths: &[FileDiffSummary],
    groups: &[AgentChangeGroup],
) -> Vec<AgentChangeCard> {
    let mut buckets: BTreeMap<String, (String, Vec<FileDiffSummary>)> = BTreeMap::new();
    for change in paths {
        let (key, title) = agent_change_card_profile(&change.path);
        buckets
            .entry(key.to_string())
            .or_insert_with(|| (title.to_string(), Vec::new()))
            .1
            .push(change.clone());
    }

    let mut ranked = Vec::new();
    for (key, (title, mut changed_paths)) in buckets {
        changed_paths.sort_by(|left, right| left.path.cmp(&right.path));
        let touched_groups = groups
            .iter()
            .filter(|group| agent_group_touches_any_path(group, &changed_paths))
            .collect::<Vec<_>>();
        let touched_by = touched_groups
            .iter()
            .map(|group| agent_file_touch(lane, group))
            .collect::<Vec<_>>();

        let mut operations = Vec::new();
        let mut seen_operations = BTreeSet::new();
        for group in &touched_groups {
            for operation in &group.operations {
                if seen_operations.insert(operation.clone()) {
                    operations.push(operation.clone());
                }
            }
        }

        let mut tool_summaries = Vec::new();
        let mut seen_tools = BTreeSet::new();
        for group in &touched_groups {
            for tool in &group.tool_summaries {
                if seen_tools.insert(tool.clone()) {
                    tool_summaries.push(tool.clone());
                }
                if tool_summaries.len() >= 8 {
                    break;
                }
            }
            if tool_summaries.len() >= 8 {
                break;
            }
        }

        let mut score = 0u8;
        let mut reasons = Vec::new();
        for change in &changed_paths {
            let (path_score, path_reasons) = agent_review_priority_score(change, touched_by.len());
            score = score.max(path_score);
            for reason in path_reasons {
                agent_push_unique_string(&mut reasons, reason);
            }
        }
        if changed_paths.len() > 1 {
            agent_push_unique_string(&mut reasons, format!("spans {} files", changed_paths.len()));
        }
        if touched_by.len() > 1 {
            agent_push_unique_string(
                &mut reasons,
                format!("touched by {} turns or operations", touched_by.len()),
            );
        }
        if reasons.is_empty() {
            reasons.push("small localized edit".to_string());
        }

        let risk = agent_change_card_risk(score);
        let churn = agent_change_card_churn(&changed_paths);
        let first_path = changed_paths.first().map(|change| change.path.as_str());
        let focus_command = first_path
            .map(|path| format!("crabdb agent focus {lane} --file {}", agent_shell_arg(path)));
        let why_command =
            first_path.map(|path| format!("crabdb agent why {lane} {}", agent_shell_arg(path)));
        let diff_command = first_path.map(|path| {
            if changed_paths.len() == 1 {
                touched_by
                    .first()
                    .map(|touch| agent_file_diff_command(lane, touch, path))
                    .unwrap_or_else(|| {
                        format!(
                            "crabdb agent diff {lane} --file {} --patch",
                            agent_shell_arg(path)
                        )
                    })
            } else if let Some(touch) = touched_by.first() {
                touch
                    .diff_command
                    .clone()
                    .unwrap_or_else(|| format!("crabdb agent diff {lane} --patch"))
            } else {
                format!("crabdb agent diff {lane} --patch")
            }
        });
        let summary = agent_change_card_summary(&title, &changed_paths, touched_by.len(), churn);

        ranked.push((
            agent_change_card_risk_rank(&risk),
            score,
            churn,
            AgentChangeCard {
                rank: 0,
                key,
                title,
                summary,
                risk,
                reasons,
                changed_paths,
                touched_by,
                operations,
                tool_summaries,
                review_command: String::new(),
                focus_command,
                why_command,
                diff_command,
            },
        ));
    }

    ranked.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.3.title.cmp(&right.3.title))
    });
    ranked
        .into_iter()
        .enumerate()
        .map(|(index, (_, _, _, mut card))| {
            card.rank = index + 1;
            card.review_command = format!("crabdb agent change {lane} {}", card.rank);
            card
        })
        .collect()
}

fn agent_changes_next(lane: &str, cards: &[AgentChangeCard]) -> StatusSuggestion {
    if let Some(card) = cards.first() {
        return StatusSuggestion {
            command: card.review_command.clone(),
            reason: format!("review the highest-priority change card: {}", card.title),
        };
    }
    StatusSuggestion {
        command: format!("crabdb agent next {lane}"),
        reason: "ask CrabDB for the next useful action".to_string(),
    }
}

fn agent_changes_suggestions(
    lane: &str,
    cards: &[AgentChangeCard],
    next: &StatusSuggestion,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    if let Some(card) = cards.first() {
        if let Some(command) = &card.focus_command {
            agent_push_suggestion(
                &mut suggestions,
                command.clone(),
                "inspect the first file in the highest-priority card",
            );
        }
        if let Some(command) = &card.diff_command {
            agent_push_suggestion(
                &mut suggestions,
                command.clone(),
                "show the focused patch for the highest-priority card",
            );
        }
    }
    agent_push_suggestion(
        &mut suggestions,
        agent_turn_diff_command(lane, None, None, false),
        "inspect the most recent turn diff",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview applying the whole agent task",
    );
    suggestions
}

fn agent_changes_summary(
    view: &AgentTaskViewReport,
    grouping: &str,
    cards: &[AgentChangeCard],
    groups: &[AgentChangeGroup],
) -> String {
    if view.task.changed_paths.is_empty() {
        return "No changed files recorded for this agent task.".to_string();
    }
    format!(
        "{} changed file(s) are grouped into {} review card(s), with {} {} checkpoint group(s) available for detail.",
        view.task.changed_paths.len(),
        cards.len(),
        groups.len(),
        grouping
    )
}

fn agent_select_change_card<'a>(
    cards: &'a [AgentChangeCard],
    selector: &str,
) -> Result<&'a AgentChangeCard> {
    let selector = selector.trim();
    if cards.is_empty() {
        return Err(Error::InvalidInput(
            "agent task has no change cards because no changed files are recorded".to_string(),
        ));
    }
    if selector.is_empty() || matches!(selector, "1" | "first" | "top" | "latest") {
        return Ok(&cards[0]);
    }
    if let Ok(rank) = selector.parse::<usize>() {
        return cards.iter().find(|card| card.rank == rank).ok_or_else(|| {
            Error::InvalidInput(format!(
                "agent change card rank `{rank}` was not found; available ranks: {}",
                agent_change_card_selector_list(cards)
            ))
        });
    }

    let normalized = normalize_agent_change_selector(selector);
    cards
        .iter()
        .find(|card| {
            normalize_agent_change_selector(&card.key) == normalized
                || normalize_agent_change_selector(&card.title) == normalized
                || card
                    .title
                    .to_ascii_lowercase()
                    .contains(&selector.to_ascii_lowercase())
        })
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "agent change card `{selector}` was not found; available cards: {}",
                agent_change_card_selector_list(cards)
            ))
        })
}

fn normalize_agent_change_selector(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn agent_change_card_selector_list(cards: &[AgentChangeCard]) -> String {
    cards
        .iter()
        .map(|card| format!("{} ({})", card.rank, card.key))
        .collect::<Vec<_>>()
        .join(", ")
}

fn agent_change_set_summary(
    card: &AgentChangeCard,
    groups: &[AgentChangeGroup],
    files: &[AgentFileEntry],
    patches: bool,
) -> String {
    let file_word = if files.len() == 1 { "file" } else { "files" };
    let group_word = if groups.len() == 1 {
        "timeline item"
    } else {
        "timeline items"
    };
    let patch_note = if patches {
        " Focused patches are included."
    } else {
        " Run with `--patch` to include focused patches."
    };
    format!(
        "{}: {:?} risk, {} {file_word}, {} {group_word}.{}",
        card.title,
        card.risk,
        files.len(),
        groups.len(),
        patch_note
    )
}

fn agent_change_set_suggestions(
    lane: &str,
    card: &AgentChangeCard,
    next: &StatusSuggestion,
    patches: bool,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    if patches {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent change {lane} {}", card.key),
            "return to the compact view of this change set",
        );
    }
    if let Some(path) = card
        .changed_paths
        .first()
        .map(|change| change.path.as_str())
    {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent why {lane} {}", agent_shell_arg(path)),
            "explain the first file in this change set",
        );
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent timeline {lane}"),
        "see where this change set fits in the whole task timeline",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane}"),
        "return to all change cards",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview applying the whole agent task",
    );
    suggestions
}

fn agent_file_summary(
    path: &str,
    matched: bool,
    change: Option<&FileDiffSummary>,
    groups: &[AgentChangeGroup],
    cards: &[AgentChangeCard],
    patches: bool,
) -> String {
    if !matched {
        return format!("`{path}` is not recorded as changed in this agent task.");
    }
    let change_phrase = change
        .map(|change| {
            format!(
                "{:?} (+{} -{})",
                change.kind, change.additions, change.deletions
            )
        })
        .unwrap_or_else(|| "changed".to_string());
    let group_word = if groups.len() == 1 {
        "timeline item"
    } else {
        "timeline items"
    };
    let card_word = if cards.len() == 1 {
        "change set"
    } else {
        "change sets"
    };
    let patch_note = if patches {
        " Focused patch is included."
    } else {
        " Run with `--patch` to include the focused patch."
    };
    format!(
        "`{path}` is {change_phrase}, appears in {} {card_word}, and was touched by {} {group_word}.{}",
        cards.len(),
        groups.len(),
        patch_note
    )
}

fn agent_file_next(
    lane: &str,
    path: &str,
    matched: bool,
    patches: bool,
    first_card: Option<&AgentChangeCard>,
) -> StatusSuggestion {
    if !matched {
        return StatusSuggestion {
            command: format!("crabdb agent files {lane}"),
            reason: "list files this agent task actually changed".to_string(),
        };
    }
    if !patches {
        return StatusSuggestion {
            command: format!("crabdb agent file {lane} {} --patch", agent_shell_arg(path)),
            reason: "show the focused patch for this file".to_string(),
        };
    }
    if let Some(card) = first_card {
        return StatusSuggestion {
            command: format!("crabdb agent change {lane} {}", card.key),
            reason: "review the full change set containing this file".to_string(),
        };
    }
    StatusSuggestion {
        command: format!("crabdb agent why {lane} {}", agent_shell_arg(path)),
        reason: "explain the prompt, tools, and checkpoint behind this file".to_string(),
    }
}

fn agent_file_suggestions(
    lane: &str,
    path: &str,
    matched: bool,
    patches: bool,
    cards: &[AgentChangeCard],
    next: &StatusSuggestion,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    if matched {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent why {lane} {}", agent_shell_arg(path)),
            "explain the prompt, tools, and checkpoint behind this file",
        );
        if let Some(card) = cards.first() {
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent change {lane} {}", card.key),
                "review the full change set containing this file",
            );
            if !patches {
                agent_push_suggestion(
                    &mut suggestions,
                    format!("crabdb agent change {lane} {} --patch", card.key),
                    "review focused patches for the whole change set",
                );
            }
        }
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent files {lane}"),
        "return to all files changed by this task",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent timeline {lane}"),
        "see when this file changed in the task timeline",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview applying the whole agent task",
    );
    suggestions
}

fn agent_delta_summary(
    task: &AgentTaskReport,
    mode: &str,
    group: Option<&AgentChangeGroup>,
    file_filter: Option<&str>,
    matched: bool,
    changed_paths: &[FileDiffSummary],
    patches: bool,
) -> String {
    let Some(group) = group else {
        return format!(
            "{} has no completed turn or operation delta yet.",
            agent_task_label(task)
        );
    };
    if let Some(path) = file_filter {
        if !matched {
            return format!(
                "Latest {mode} {} did not change `{path}`. Use `agent file` to inspect the full task history for that path.",
                group.index
            );
        }
        let churn = changed_paths
            .iter()
            .map(|file| file.additions + file.deletions)
            .sum::<u64>();
        return format!(
            "Latest {mode} {} changed `{path}` with {} changed line(s). {}",
            group.index,
            churn,
            if patches {
                "Patch included."
            } else {
                "Add `--patch` to see exact hunks."
            }
        );
    }
    let churn = changed_paths
        .iter()
        .map(|file| file.additions + file.deletions)
        .sum::<u64>();
    format!(
        "Latest {mode} {} changed {} file(s) with {} changed line(s). {}",
        group.index,
        changed_paths.len(),
        churn,
        if patches {
            "Patch included."
        } else {
            "Add `--patch` to see exact hunks."
        }
    )
}

fn agent_delta_next(
    lane: &str,
    mode: &str,
    group: Option<&AgentChangeGroup>,
    file_filter: Option<&str>,
    matched: bool,
    patches: bool,
    fallback: &StatusSuggestion,
) -> StatusSuggestion {
    if group.is_none() {
        return StatusSuggestion {
            command: format!("crabdb agent changes {lane}"),
            reason: "inspect any recorded task-level changes".to_string(),
        };
    }
    if let Some(path) = file_filter {
        if !matched {
            return StatusSuggestion {
                command: format!("crabdb agent file {lane} {}", agent_shell_arg(path)),
                reason: "inspect the full task history for this file".to_string(),
            };
        }
    }
    if !patches {
        let mode_flag = if mode == "operation" {
            " --by-operation"
        } else {
            ""
        };
        let file_arg = file_filter
            .map(|path| format!(" --file {}", agent_shell_arg(path)))
            .unwrap_or_default();
        return StatusSuggestion {
            command: format!("crabdb agent delta {lane}{mode_flag}{file_arg} --patch"),
            reason: "show the exact patch for the newest agent delta".to_string(),
        };
    }
    fallback.clone()
}

fn agent_delta_suggestions(
    lane: &str,
    mode: &str,
    group: Option<&AgentChangeGroup>,
    file_filter: Option<&str>,
    matched: bool,
    patches: bool,
    changed_paths: &[FileDiffSummary],
    next: &StatusSuggestion,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    if let Some(group) = group {
        match group.kind.as_str() {
            "turn" => {
                agent_push_suggestion(
                    &mut suggestions,
                    format!("crabdb agent turn {lane} {}", group.index),
                    "inspect the prompt, response, tools, and checkpoint for this turn",
                );
                agent_push_suggestion(
                    &mut suggestions,
                    agent_turn_diff_command(lane, Some(group.index), None, true),
                    "show the raw patch for this turn",
                );
            }
            _ => {
                if let Some(operation) = &group.operation_id {
                    agent_push_suggestion(
                        &mut suggestions,
                        format!(
                            "crabdb agent diff {lane} --operation {} --patch",
                            operation.0
                        ),
                        "show the raw patch for this recorded operation",
                    );
                }
            }
        }
        if let Some(path) = file_filter {
            let command = if matched && !patches {
                format!(
                    "crabdb agent delta {lane}{} --file {} --patch",
                    if mode == "operation" {
                        " --by-operation"
                    } else {
                        ""
                    },
                    agent_shell_arg(path)
                )
            } else {
                format!("crabdb agent file {lane} {} --patch", agent_shell_arg(path))
            };
            agent_push_suggestion(
                &mut suggestions,
                command,
                "stay focused on this file with provenance and patch context",
            );
        } else if let Some(path) = changed_paths.first() {
            agent_push_suggestion(
                &mut suggestions,
                format!(
                    "crabdb agent file {lane} {} --patch",
                    agent_shell_arg(&path.path)
                ),
                "inspect the first changed file from the newest delta",
            );
        }
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent timeline {lane}"),
        "see where this delta sits in the full task timeline",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane}"),
        "group the full task into high-level change cards",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview applying the whole agent task safely",
    );
    suggestions
}

fn agent_groups_after_review_marker(
    groups: Vec<AgentChangeGroup>,
    reviewed: Option<&AgentReviewMarker>,
    head_change: &ChangeId,
) -> Vec<AgentChangeGroup> {
    let Some(reviewed) = reviewed else {
        return groups;
    };
    if &reviewed.checkpoint == head_change {
        return Vec::new();
    }
    let Some(index) = groups.iter().position(|group| {
        group.after_change.as_ref() == Some(&reviewed.checkpoint)
            || group.checkpoint.as_ref() == Some(&reviewed.checkpoint)
            || group.operation_id.as_ref() == Some(&reviewed.checkpoint)
    }) else {
        return groups;
    };
    groups.into_iter().skip(index + 1).collect()
}

fn agent_new_status(
    reviewed: Option<&AgentReviewMarker>,
    changed_paths: &[FileDiffSummary],
) -> String {
    if changed_paths.is_empty() {
        "up_to_date".to_string()
    } else if reviewed.is_some() {
        "new_changes".to_string()
    } else {
        "unreviewed".to_string()
    }
}

fn agent_new_summary(
    task: &AgentTaskReport,
    status: &str,
    reviewed: Option<&AgentReviewMarker>,
    file_filter: Option<&str>,
    matched: bool,
    changed_paths: &[FileDiffSummary],
    patches: bool,
) -> String {
    if let Some(path) = file_filter {
        if !matched {
            return match reviewed {
                Some(marker) => format!(
                    "No new changes touched `{path}` since reviewed checkpoint `{}`.",
                    marker.checkpoint.0
                ),
                None => format!(
                    "`{path}` has no recorded changes in {}.",
                    agent_task_label(task)
                ),
            };
        }
    }
    let changed_lines = changed_paths
        .iter()
        .map(|file| file.additions + file.deletions)
        .sum::<u64>();
    let scope = file_filter
        .map(|path| format!(" for `{path}`"))
        .unwrap_or_default();
    match status {
        "up_to_date" => reviewed
            .map(|marker| {
                format!(
                    "{} has no new changes{scope} since reviewed checkpoint `{}`.",
                    agent_task_label(task),
                    marker.checkpoint.0
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "{} has no recorded changes{scope} yet.",
                    agent_task_label(task)
                )
            }),
        "new_changes" => {
            let checkpoint = reviewed
                .map(|marker| marker.checkpoint.0.as_str())
                .unwrap_or("task start");
            format!(
                "{} has {} new changed file(s){scope} and {} changed line(s) since `{checkpoint}`. {}",
                agent_task_label(task),
                changed_paths.len(),
                changed_lines,
                if patches {
                    "Patch included."
                } else {
                    "Add `--patch` to see exact hunks."
                }
            )
        }
        _ => format!(
            "{} has not been marked reviewed yet; showing {} changed file(s){scope} from the task start. {}",
            agent_task_label(task),
            changed_paths.len(),
            if patches {
                "Patch included."
            } else {
                "Run `agent done` when this baseline has been inspected."
            }
        ),
    }
}

fn agent_new_next(
    lane: &str,
    status: &str,
    file_filter: Option<&str>,
    matched: bool,
    patches: bool,
) -> StatusSuggestion {
    if file_filter.is_some() && !matched {
        return StatusSuggestion {
            command: format!("crabdb agent new {lane}"),
            reason: "return to all new task changes since the reviewed checkpoint".to_string(),
        };
    }
    match status {
        "up_to_date" => StatusSuggestion {
            command: format!("crabdb agent status"),
            reason: "return to the latest task status".to_string(),
        },
        "new_changes" | "unreviewed" if !patches => {
            let file_arg = file_filter
                .map(|path| format!(" --file {}", agent_shell_arg(path)))
                .unwrap_or_default();
            StatusSuggestion {
                command: format!("crabdb agent new {lane}{file_arg} --patch"),
                reason: "show the exact patch for changes not yet marked reviewed".to_string(),
            }
        }
        _ => StatusSuggestion {
            command: format!("crabdb agent done {lane}"),
            reason: "mark the current checkpoint reviewed after inspection".to_string(),
        },
    }
}

fn agent_new_suggestions(
    lane: &str,
    status: &str,
    file_filter: Option<&str>,
    matched: bool,
    patches: bool,
    changed_paths: &[FileDiffSummary],
    next: &StatusSuggestion,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    if matches!(status, "new_changes" | "unreviewed") {
        if !patches {
            let file_arg = file_filter
                .map(|path| format!(" --file {}", agent_shell_arg(path)))
                .unwrap_or_default();
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent new {lane}{file_arg} --patch"),
                "inspect the new patch before marking it reviewed",
            );
        }
        if matched {
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent done {lane}"),
                "set the current checkpoint as the reviewed baseline",
            );
        }
    }
    if let Some(path) = file_filter {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent file {lane} {} --patch", agent_shell_arg(path)),
            "inspect full task provenance for this file",
        );
    } else if let Some(path) = changed_paths.first() {
        agent_push_suggestion(
            &mut suggestions,
            format!(
                "crabdb agent file {lane} {} --patch",
                agent_shell_arg(&path.path)
            ),
            "inspect the first new changed file with provenance",
        );
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent delta {lane} --patch"),
        "inspect only the newest completed turn or operation",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane}"),
        "review the whole task as high-level change cards",
    );
    suggestions
}

fn agent_mark_reviewed_summary(
    task: &AgentTaskReport,
    previous: Option<&AgentReviewMarker>,
    marker: &AgentReviewMarker,
) -> String {
    match previous {
        Some(previous) if previous.checkpoint == marker.checkpoint => format!(
            "{} was already reviewed at checkpoint `{}`; refreshed the reviewed marker.",
            agent_task_label(task),
            marker.checkpoint.0
        ),
        Some(previous) => format!(
            "{} marked reviewed at checkpoint `{}`; previous reviewed checkpoint was `{}`.",
            agent_task_label(task),
            marker.checkpoint.0,
            previous.checkpoint.0
        ),
        None => format!(
            "{} marked reviewed at checkpoint `{}`.",
            agent_task_label(task),
            marker.checkpoint.0
        ),
    }
}

fn agent_mark_reviewed_suggestions(lane: &str) -> Vec<StatusSuggestion> {
    vec![
        StatusSuggestion {
            command: format!("crabdb agent new {lane}"),
            reason: "confirm there are no new unreviewed changes".to_string(),
        },
        StatusSuggestion {
            command: format!("crabdb agent status"),
            reason: "return to the latest task status and next action".to_string(),
        },
    ]
}

fn agent_timeline_items(
    lane: &str,
    view: &AgentTaskViewReport,
    groups: &[AgentChangeGroup],
) -> Vec<AgentTimelineItem> {
    groups
        .iter()
        .map(|group| {
            let (message_count, event_count) = agent_timeline_counts(view, group);
            AgentTimelineItem {
                kind: group.kind.clone(),
                index: group.index,
                id: group.id.clone(),
                title: agent_timeline_item_title(group),
                status: group.status.clone(),
                prompt_preview: group.prompt_preview.clone(),
                assistant_preview: group.assistant_preview.clone(),
                before_change: group.before_change.clone(),
                after_change: group.after_change.clone(),
                checkpoint: group.checkpoint.clone(),
                operations: group.operations.clone(),
                changed_paths: group.changed_paths.clone(),
                tool_summaries: group.tool_summaries.clone(),
                message_count,
                event_count,
                view_command: agent_timeline_view_command(lane, group),
                diff_command: agent_timeline_diff_command(lane, group),
                rewind_before_command: agent_timeline_rewind_before_command(lane, group),
            }
        })
        .collect()
}

fn agent_timeline_counts(view: &AgentTaskViewReport, group: &AgentChangeGroup) -> (usize, usize) {
    let Some(transcript) = &view.transcript else {
        return (0, 0);
    };
    let turn = group
        .turn_id
        .as_ref()
        .and_then(|turn_id| {
            transcript
                .turns
                .iter()
                .find(|turn| turn.turn.turn_id == *turn_id)
        })
        .or_else(|| {
            group.operation_id.as_ref().and_then(|operation_id| {
                transcript
                    .turns
                    .iter()
                    .find(|turn| turn_contains_operation(turn, operation_id))
            })
        });
    let Some(turn) = turn else {
        return (0, 0);
    };
    let event_count = if group.kind == "operation" {
        group
            .operation_id
            .as_ref()
            .map(|operation_id| {
                turn.events
                    .iter()
                    .filter(|event| event.change_id.as_ref() == Some(operation_id))
                    .count()
            })
            .unwrap_or(0)
    } else {
        turn.events.len()
    };
    (turn.messages.len(), event_count)
}

fn agent_timeline_item_title(group: &AgentChangeGroup) -> String {
    match group.kind.as_str() {
        "turn" => group
            .prompt_preview
            .as_deref()
            .map(|prompt| format!("Turn {}: {prompt}", group.index))
            .unwrap_or_else(|| format!("Turn {}", group.index)),
        _ => group
            .status
            .as_deref()
            .map(|status| format!("Operation {}: {status}", group.index))
            .unwrap_or_else(|| format!("Operation {}", group.index)),
    }
}

fn agent_timeline_view_command(lane: &str, group: &AgentChangeGroup) -> Option<String> {
    match group.kind.as_str() {
        "turn" => Some(format!("crabdb agent turn {lane} {}", group.index)),
        _ => group
            .operation_id
            .as_ref()
            .map(|operation| format!("crabdb agent diff {lane} --operation {}", operation.0)),
    }
}

fn agent_timeline_diff_command(lane: &str, group: &AgentChangeGroup) -> Option<String> {
    match group.kind.as_str() {
        "turn" if group.after_change.is_some() => Some(agent_turn_diff_command(
            lane,
            Some(group.index),
            None,
            false,
        )),
        "operation" => group
            .operation_id
            .as_ref()
            .map(|operation| format!("crabdb agent diff {lane} --operation {}", operation.0)),
        _ => None,
    }
}

fn agent_timeline_rewind_before_command(lane: &str, group: &AgentChangeGroup) -> Option<String> {
    group.before_change.as_ref()?;
    match group.kind.as_str() {
        "turn" => Some(format!(
            "crabdb agent rewind {lane} --to before-turn:{}",
            group.index
        )),
        _ => group
            .before_change
            .as_ref()
            .map(|change| format!("crabdb agent rewind {lane} --to {}", change.0)),
    }
}

fn agent_timeline_summary(
    view: &AgentTaskViewReport,
    mode: &str,
    items: &[AgentTimelineItem],
) -> String {
    if items.is_empty() {
        return format!(
            "{} has no recorded turn or operation checkpoints yet.",
            agent_task_label(&view.task)
        );
    }
    let changed_files = view.task.changed_paths.len();
    let tool_events = items
        .iter()
        .map(|item| item.tool_summaries.len())
        .sum::<usize>();
    let checkpoint_count = items
        .iter()
        .filter(|item| item.checkpoint.is_some())
        .count();
    format!(
        "{} has {} {} timeline item(s), {} checkpoint(s), {} changed file(s), and {} captured tool summary item(s).",
        agent_task_label(&view.task),
        items.len(),
        mode,
        checkpoint_count,
        changed_files,
        tool_events
    )
}

fn agent_timeline_suggestions(
    lane: &str,
    mode: &str,
    items: &[AgentTimelineItem],
) -> Vec<StatusSuggestion> {
    let mut suggestions = Vec::new();
    if mode != "operation" {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent timeline {lane} --by-operation"),
            "zoom in from prompt turns to recorded CrabDB operations",
        );
    }
    if let Some(command) = items
        .iter()
        .rev()
        .find_map(|item| item.view_command.clone())
    {
        agent_push_suggestion(
            &mut suggestions,
            command,
            "inspect the latest timeline item",
        );
    }
    if let Some(command) = items
        .iter()
        .rev()
        .find_map(|item| item.diff_command.clone())
    {
        agent_push_suggestion(
            &mut suggestions,
            format!("{command} --patch"),
            "inspect the latest timeline item patch",
        );
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane}"),
        "switch to high-level change cards grouped by user intent",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent land {lane} --dry-run"),
        "preview applying the whole agent task",
    );
    suggestions
}

fn agent_change_card_profile(path: &str) -> (&'static str, &'static str) {
    if path == "README.md" || path.starts_with("docs/") {
        ("docs", "Docs and getting-started")
    } else if path == "Cargo.toml" || path == "Cargo.lock" || path.ends_with("/Cargo.toml") {
        ("build", "Build and dependencies")
    } else if path.starts_with("crates/crabdb/src/cli/") {
        ("cli", "CLI workflow")
    } else if path.starts_with("crates/crabdb/src/mcp/") {
        ("mcp", "MCP and editor surfaces")
    } else if path == "crates/crabdb/src/acp.rs" || path.contains("/acp") {
        ("acp", "ACP agent integration")
    } else if path.starts_with("crates/crabdb/src/server/") || path == "crates/crabdb/src/server.rs"
    {
        ("api", "HTTP API surface")
    } else if path.starts_with("crates/crabdb/src/db/storage/") {
        ("storage", "Storage and indexing")
    } else if path.starts_with("crates/crabdb/src/db/merge/") {
        ("merge", "Merge and apply safety")
    } else if path.starts_with("crates/crabdb/src/db/lane/") {
        ("lane", "Lane and workdir mechanics")
    } else if path.starts_with("crates/crabdb/src/db/") {
        ("db", "CrabDB behavior")
    } else if path.starts_with("crates/crabdb/tests/") || path.ends_with("tests.rs") {
        ("tests", "Tests and regression coverage")
    } else if path.starts_with("crates/core/") {
        ("core", "Core library behavior")
    } else {
        ("workspace", "Workspace changes")
    }
}

fn agent_group_touches_any_path(group: &AgentChangeGroup, paths: &[FileDiffSummary]) -> bool {
    paths.iter().any(|path| {
        group
            .changed_paths
            .iter()
            .any(|change| agent_file_matches_path(change, &path.path))
    })
}

fn agent_change_card_summary(
    title: &str,
    paths: &[FileDiffSummary],
    touch_count: usize,
    churn: u64,
) -> String {
    let first_path = paths
        .first()
        .map(|change| format!("`{}`", change.path))
        .unwrap_or_else(|| "no files".to_string());
    let file_phrase = if paths.len() == 1 {
        first_path
    } else {
        format!(
            "{first_path} and {} more file(s)",
            paths.len().saturating_sub(1)
        )
    };
    let touch_phrase = if touch_count == 0 {
        "no captured turn".to_string()
    } else if touch_count == 1 {
        "1 turn or operation".to_string()
    } else {
        format!("{touch_count} turns or operations")
    };
    format!("{title}: {file_phrase}, {churn} changed line(s), touched by {touch_phrase}.")
}

fn agent_change_card_churn(paths: &[FileDiffSummary]) -> u64 {
    paths
        .iter()
        .map(|path| path.additions + path.deletions)
        .sum()
}

fn agent_change_card_risk(score: u8) -> AgentRiskLevel {
    if score >= 70 {
        AgentRiskLevel::High
    } else if score >= 35 {
        AgentRiskLevel::Medium
    } else {
        AgentRiskLevel::Low
    }
}

fn agent_change_card_risk_rank(risk: &AgentRiskLevel) -> u8 {
    match risk {
        AgentRiskLevel::Blocking => 4,
        AgentRiskLevel::High => 3,
        AgentRiskLevel::Medium => 2,
        AgentRiskLevel::Low => 1,
    }
}

fn agent_push_unique_string(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn agent_review_priorities(
    lane: &str,
    paths: &[FileDiffSummary],
    groups: &[AgentChangeGroup],
) -> Vec<AgentReviewPriority> {
    let mut priorities = paths
        .iter()
        .map(|change| {
            let touched_by = groups
                .iter()
                .filter(|group| {
                    group
                        .changed_paths
                        .iter()
                        .any(|file| agent_file_matches_path(file, &change.path))
                })
                .map(|group| agent_file_touch_for_path(lane, group, &change.path))
                .collect::<Vec<_>>();
            let (score, reasons) = agent_review_priority_score(change, touched_by.len());
            let file_arg = agent_shell_arg(&change.path);
            let diff_command = touched_by
                .first()
                .map(|touch| match touch.kind.as_str() {
                    "turn" => {
                        agent_turn_diff_command(lane, Some(touch.index), Some(&change.path), true)
                    }
                    _ => touch
                        .operation_id
                        .as_ref()
                        .map(|operation| {
                            format!(
                                "crabdb agent diff {lane} --operation {} --file {file_arg} --patch",
                                operation.0
                            )
                        })
                        .unwrap_or_else(|| {
                            format!("crabdb agent diff {lane} --file {file_arg} --patch")
                        }),
                })
                .or_else(|| {
                    Some(format!(
                        "crabdb agent diff {lane} --file {file_arg} --patch"
                    ))
                });
            AgentReviewPriority {
                rank: 0,
                change: change.clone(),
                score,
                reasons,
                touched_by,
                why_command: format!("crabdb agent why {lane} {file_arg}"),
                diff_command,
            }
        })
        .collect::<Vec<_>>();
    priorities.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| {
                let right_churn = right.change.additions + right.change.deletions;
                let left_churn = left.change.additions + left.change.deletions;
                right_churn.cmp(&left_churn)
            })
            .then_with(|| left.change.path.cmp(&right.change.path))
    });
    for (index, priority) in priorities.iter_mut().enumerate() {
        priority.rank = index + 1;
    }
    priorities
}

fn agent_review_priority_score(change: &FileDiffSummary, touch_count: usize) -> (u8, Vec<String>) {
    let mut score = 10u8;
    let mut reasons = Vec::new();
    let churn = change.additions + change.deletions;
    if churn >= 500 {
        score = score.saturating_add(30);
        reasons.push(format!(
            "large edit (+{} -{})",
            change.additions, change.deletions
        ));
    } else if churn >= 100 {
        score = score.saturating_add(20);
        reasons.push(format!(
            "moderate edit (+{} -{})",
            change.additions, change.deletions
        ));
    } else if churn >= 20 {
        score = score.saturating_add(10);
        reasons.push(format!(
            "non-trivial edit (+{} -{})",
            change.additions, change.deletions
        ));
    }

    if agent_changed_manifest_or_api_surface(std::slice::from_ref(change)) {
        score = score.saturating_add(30);
        reasons.push("package, lockfile, or public API surface".to_string());
    }

    match &change.kind {
        FileChangeKind::Added => {
            score = score.saturating_add(15);
            reasons.push("new file".to_string());
        }
        FileChangeKind::Deleted => {
            score = score.saturating_add(20);
            reasons.push("deleted file".to_string());
        }
        FileChangeKind::Renamed | FileChangeKind::TypeChanged => {
            score = score.saturating_add(20);
            reasons.push("structural file change".to_string());
        }
        FileChangeKind::Modified => {}
    }

    if agent_path_looks_like_test(&change.path) {
        score = score.saturating_add(10);
        reasons.push("test or validation file changed".to_string());
    }
    if touch_count > 1 {
        score = score.saturating_add(15);
        reasons.push(format!("touched by {touch_count} turns or operations"));
    }
    if reasons.is_empty() {
        reasons.push("changed by the agent task".to_string());
    }
    (score.min(100), reasons)
}

fn agent_report_markdown(
    view: &AgentTaskViewReport,
    story: &AgentStoryReport,
    risk: &AgentRiskReport,
    changes: &AgentChangesReport,
    next: &StatusSuggestion,
) -> String {
    let mut out = String::new();
    let title = agent_task_label(&view.task);
    out.push_str(&format!("# Agent Task Report: {title}\n\n"));
    out.push_str("## Summary\n\n");
    out.push_str(&format!("- Status: {:?}\n", view.task.status));
    out.push_str(&format!(
        "- Readiness: {}{}\n",
        view.review.readiness.status,
        if view.review.readiness.ready {
            " (ready)"
        } else {
            ""
        }
    ));
    out.push_str(&format!(
        "- Changed files: {}\n",
        view.task.changed_paths.len()
    ));
    out.push_str(&format!(
        "- Turns: {}  Tool events: {}\n",
        view.task.turns, view.task.tool_events
    ));
    if let Some(checkpoint) = &view.task.latest_checkpoint {
        out.push_str(&format!("- Latest checkpoint: `{}`\n", checkpoint.0));
    }
    out.push_str(&format!(
        "- Risk: {:?} ({}/100)\n\n",
        risk.level, risk.score
    ));
    out.push_str(&format!("{}\n\n", story.summary));

    out.push_str("## Next Action\n\n");
    out.push_str(&format!("```sh\n{}\n```\n\n", next.command));
    out.push_str(&format!("{}\n\n", next.reason));

    if !risk.reasons.is_empty() {
        out.push_str("## Risk Notes\n\n");
        for reason in &risk.reasons {
            out.push_str(&format!(
                "- [{}] {}: {}\n",
                reason.severity, reason.code, reason.message
            ));
        }
        out.push('\n');
    }

    if !view.review.readiness.blockers.is_empty() {
        out.push_str("## Blockers\n\n");
        for blocker in &view.review.readiness.blockers {
            out.push_str(&format!("- {}: {}\n", blocker.code, blocker.message));
        }
        out.push('\n');
    }
    if !view.review.readiness.warnings.is_empty() {
        out.push_str("## Warnings\n\n");
        for warning in &view.review.readiness.warnings {
            out.push_str(&format!("- {}: {}\n", warning.code, warning.message));
        }
        out.push('\n');
    }

    out.push_str("## Changes\n\n");
    if view.task.changed_paths.is_empty() {
        out.push_str("No changed files recorded.\n\n");
    } else {
        for path in &view.task.changed_paths {
            out.push_str(&format!(
                "- {:?} `{}` (+{} -{})\n",
                path.kind, path.path, path.additions, path.deletions
            ));
        }
        out.push('\n');
    }

    if !story.turn_summaries.is_empty() {
        out.push_str("## Turns\n\n");
        for turn in &story.turn_summaries {
            let label = turn
                .prompt_preview
                .as_deref()
                .or(turn.outcome_preview.as_deref())
                .unwrap_or(&turn.id);
            out.push_str(&format!(
                "{}. {} ({} changed file(s))\n",
                turn.index,
                label,
                turn.changed_paths.len()
            ));
            if let Some(checkpoint) = &turn.checkpoint {
                out.push_str(&format!("   - Checkpoint: `{}`\n", checkpoint.0));
            }
            for tool in &turn.tool_summaries {
                out.push_str(&format!("   - Tool: {tool}\n"));
            }
        }
        out.push('\n');
    }

    if !changes.groups.is_empty() {
        out.push_str("## Review Commands\n\n");
        out.push_str(&format!("```sh\ncrabdb agent changes {}\n", view.task.lane));
        out.push_str(&format!(
            "{}\n",
            agent_turn_diff_command(&view.task.lane, None, None, true)
        ));
        out.push_str(&format!("crabdb agent why {} <path>\n", view.task.lane));
        out.push_str("```\n");
    }
    out
}

fn agent_receipt_markdown(
    report: &AgentReviewBundleReport,
    validation: &[LaneTestSummary],
) -> String {
    let mut out = String::new();
    let title = agent_task_label(&report.task);
    out.push_str(&format!("# Agent Task Receipt: {title}\n\n"));
    out.push_str("## Summary\n\n");
    out.push_str(&format!("- Status: {:?}\n", report.task.status));
    out.push_str(&format!(
        "- Readiness: {}{}\n",
        report.readiness_status,
        if report.ready_to_apply {
            " (ready)"
        } else {
            ""
        }
    ));
    out.push_str(&format!(
        "- Changed files: {}\n",
        report.task.changed_paths.len()
    ));
    out.push_str(&format!(
        "- Turns: {}  Tool events: {}\n",
        report.task.turns, report.task.tool_events
    ));
    if let Some(checkpoint) = &report.task.latest_checkpoint {
        out.push_str(&format!("- Latest checkpoint: `{}`\n", checkpoint.0));
    }
    out.push_str(&format!(
        "- Risk: {:?} ({}/100)\n\n",
        report.risk.level, report.risk.score
    ));
    out.push_str(&format!("{}\n\n", report.story.summary));

    out.push_str("## Next Command\n\n");
    out.push_str(&format!("```sh\n{}\n```\n\n", report.next.command));
    out.push_str(&format!("{}\n\n", report.next.reason));

    out.push_str("## Validation\n\n");
    if validation.is_empty() {
        out.push_str("No test or eval gate has been recorded for this task.\n\n");
    } else {
        for gate in validation {
            let suite = gate.suite.as_deref().unwrap_or("default");
            let result = if gate.success { "passed" } else { "failed" };
            out.push_str(&format!(
                "- {} `{}` {}: `{}`",
                gate.kind,
                suite,
                result,
                shell_join(&gate.command)
            ));
            if let Some(code) = gate.exit_code {
                out.push_str(&format!(" (exit {code})"));
            }
            if let Some(score) = gate.score {
                out.push_str(&format!(" score {score}"));
            }
            if let Some(threshold) = gate.threshold {
                out.push_str(&format!(" threshold {threshold}"));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    if !report.review.readiness.blockers.is_empty() {
        out.push_str("## Blockers\n\n");
        for blocker in &report.review.readiness.blockers {
            out.push_str(&format!("- {}: {}\n", blocker.code, blocker.message));
        }
        out.push('\n');
    }
    if !report.review.readiness.warnings.is_empty() {
        out.push_str("## Warnings\n\n");
        for warning in &report.review.readiness.warnings {
            out.push_str(&format!("- {}: {}\n", warning.code, warning.message));
        }
        out.push('\n');
    }

    if !report.risk.reasons.is_empty() {
        out.push_str("## Risk Notes\n\n");
        for reason in &report.risk.reasons {
            out.push_str(&format!(
                "- [{}] {}: {}\n",
                reason.severity, reason.code, reason.message
            ));
        }
        out.push('\n');
    }

    out.push_str("## Changed Files\n\n");
    if report.task.changed_paths.is_empty() {
        out.push_str("No changed files recorded.\n\n");
    } else {
        for path in &report.task.changed_paths {
            out.push_str(&format!(
                "- {:?} `{}` (+{} -{})\n",
                path.kind, path.path, path.additions, path.deletions
            ));
        }
        out.push('\n');
    }

    if !report.story.turn_summaries.is_empty() {
        out.push_str("## Turns\n\n");
        for turn in &report.story.turn_summaries {
            let label = turn
                .prompt_preview
                .as_deref()
                .or(turn.outcome_preview.as_deref())
                .unwrap_or(&turn.id);
            out.push_str(&format!(
                "{}. {} ({} changed file(s))\n",
                turn.index,
                label,
                turn.changed_paths.len()
            ));
            if let Some(checkpoint) = &turn.checkpoint {
                out.push_str(&format!("   - Checkpoint: `{}`\n", checkpoint.0));
            }
            for tool in &turn.tool_summaries {
                out.push_str(&format!("   - Tool: {tool}\n"));
            }
        }
        out.push('\n');
    }

    if !report.story.tool_summaries.is_empty() {
        out.push_str("## Tools\n\n");
        for tool in &report.story.tool_summaries {
            out.push_str(&format!("- {tool}\n"));
        }
        out.push('\n');
    }

    out.push_str("## Useful Commands\n\n");
    out.push_str(&format!("```sh\ncrabdb agent ready {}\n", report.task.lane));
    out.push_str(&format!("crabdb agent changes {}\n", report.task.lane));
    out.push_str(&format!("crabdb agent review-plan {}\n", report.task.lane));
    out.push_str(&format!(
        "crabdb agent land {} --dry-run\n",
        report.task.lane
    ));
    out.push_str("```\n");
    out
}

fn agent_pr_title(receipt: &AgentReceiptReport) -> String {
    let title = agent_task_label(&receipt.task).trim();
    if title.is_empty() {
        format!("Apply agent task {}", receipt.task.name)
    } else if title.to_ascii_lowercase().starts_with("apply ") {
        single_line_preview(title, 72)
    } else {
        single_line_preview(&format!("Apply {title}"), 72)
    }
}

fn agent_pr_body(receipt: &AgentReceiptReport) -> String {
    let mut out = String::new();
    out.push_str("## Summary\n\n");
    out.push_str(&format!("{}\n\n", receipt.summary));
    out.push_str(&format!("- Status: {:?}\n", receipt.status));
    out.push_str(&format!(
        "- Readiness: {}{}\n",
        receipt.readiness_status,
        if receipt.ready_to_apply {
            " (ready)"
        } else {
            ""
        }
    ));
    out.push_str(&format!(
        "- Risk: {:?} ({}/100)\n",
        receipt.risk.level, receipt.risk.score
    ));
    if let Some(checkpoint) = &receipt.latest_checkpoint {
        out.push_str(&format!("- CrabDB checkpoint: `{}`\n", checkpoint.0));
    }
    out.push_str(&format!("- Agent task: `{}`\n\n", receipt.task.name));

    out.push_str("## Changes\n\n");
    if receipt.changed_paths.is_empty() {
        out.push_str("No changed files recorded.\n\n");
    } else {
        for path in &receipt.changed_paths {
            out.push_str(&format!(
                "- {:?} `{}` (+{} -{})\n",
                path.kind, path.path, path.additions, path.deletions
            ));
        }
        out.push('\n');
    }

    out.push_str("## Validation\n\n");
    if receipt.validation.is_empty() {
        out.push_str("No test or eval gate has been recorded for this task.\n\n");
    } else {
        for gate in &receipt.validation {
            let suite = gate.suite.as_deref().unwrap_or("default");
            let result = if gate.success { "passed" } else { "failed" };
            out.push_str(&format!(
                "- {} `{}` {}: `{}`",
                gate.kind,
                suite,
                result,
                shell_join(&gate.command)
            ));
            if let Some(code) = gate.exit_code {
                out.push_str(&format!(" (exit {code})"));
            }
            if let Some(score) = gate.score {
                out.push_str(&format!(" score {score}"));
            }
            if let Some(threshold) = gate.threshold {
                out.push_str(&format!(" threshold {threshold}"));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    if !receipt.risk.reasons.is_empty() {
        out.push_str("## Risk Notes\n\n");
        for reason in &receipt.risk.reasons {
            out.push_str(&format!(
                "- [{}] {}: {}\n",
                reason.severity, reason.code, reason.message
            ));
        }
        out.push('\n');
    }

    if !receipt.turns.is_empty() {
        out.push_str("## Agent Turns\n\n");
        for turn in &receipt.turns {
            let label = turn
                .prompt_preview
                .as_deref()
                .or(turn.outcome_preview.as_deref())
                .unwrap_or(&turn.id);
            out.push_str(&format!(
                "{}. {} ({} changed file(s))\n",
                turn.index,
                label,
                turn.changed_paths.len()
            ));
            if let Some(checkpoint) = &turn.checkpoint {
                out.push_str(&format!("   - Checkpoint: `{}`\n", checkpoint.0));
            }
        }
        out.push('\n');
    }

    out.push_str("## CrabDB Review\n\n");
    out.push_str(&format!(
        "```sh\ncrabdb agent receipt {}\n",
        receipt.task.lane
    ));
    out.push_str(&format!("crabdb agent ready {}\n", receipt.task.lane));
    out.push_str(&format!("crabdb agent changes {}\n", receipt.task.lane));
    out.push_str("```\n");
    out
}

fn agent_checkpoint_entry(lane: &str, group: &AgentChangeGroup) -> AgentCheckpointEntry {
    let is_turn = group.kind == "turn";
    let label = if is_turn {
        format!("Turn {}", group.index)
    } else {
        format!("Operation {}", group.index)
    };
    let before_target = group.before_change.as_ref().map(|before| {
        if is_turn {
            format!("before-turn:{}", group.index)
        } else {
            before.0.clone()
        }
    });
    let checkpoint_target = group.checkpoint.as_ref().map(|checkpoint| {
        if is_turn {
            format!("turn:{}", group.index)
        } else {
            checkpoint.0.clone()
        }
    });
    let rewind_before_command = before_target
        .as_ref()
        .map(|target| format!("crabdb agent rewind {lane} --to {target}"));
    let rewind_checkpoint_command = checkpoint_target
        .as_ref()
        .map(|target| format!("crabdb agent rewind {lane} --to {target}"));
    let diff_command = if is_turn {
        Some(agent_turn_diff_command(lane, Some(group.index), None, true))
    } else {
        group.operation_id.as_ref().map(|operation| {
            format!(
                "crabdb agent diff {lane} --operation {} --patch",
                operation.0
            )
        })
    };
    AgentCheckpointEntry {
        kind: group.kind.clone(),
        index: group.index,
        id: group.id.clone(),
        label,
        turn_id: group.turn_id.clone(),
        prompt_preview: group.prompt_preview.clone(),
        before_change: group.before_change.clone(),
        checkpoint: group.checkpoint.clone(),
        before_target,
        checkpoint_target,
        changed_paths: group.changed_paths.clone(),
        rewind_before_command,
        rewind_checkpoint_command,
        diff_command,
    }
}

fn agent_file_entry(
    lane: &str,
    change: &FileDiffSummary,
    groups: &[AgentChangeGroup],
) -> AgentFileEntry {
    let touched_by = groups
        .iter()
        .filter(|group| {
            group
                .changed_paths
                .iter()
                .any(|file| agent_file_matches_path(file, &change.path))
        })
        .map(|group| agent_file_touch_for_path(lane, group, &change.path))
        .collect::<Vec<_>>();
    let diff_command = touched_by
        .first()
        .map(|touch| agent_file_diff_command(lane, touch, &change.path))
        .or_else(|| {
            Some(format!(
                "crabdb agent diff {lane} --file {} --patch",
                agent_shell_arg(&change.path)
            ))
        });
    AgentFileEntry {
        change: change.clone(),
        touched_by,
        why_command: format!("crabdb agent why {lane} {}", agent_shell_arg(&change.path)),
        diff_command,
        report_command: format!("crabdb agent report {lane} --markdown"),
    }
}

fn agent_file_diff_command(lane: &str, touch: &AgentFileTouch, path: &str) -> String {
    let file_arg = agent_shell_arg(path);
    match touch.kind.as_str() {
        "turn" => agent_turn_diff_command(lane, Some(touch.index), Some(path), true),
        _ => touch
            .operation_id
            .as_ref()
            .map(|operation| {
                format!(
                    "crabdb agent diff {lane} --operation {} --file {file_arg} --patch",
                    operation.0
                )
            })
            .unwrap_or_else(|| format!("crabdb agent diff {lane} --file {file_arg} --patch")),
    }
}

fn agent_file_touch_for_path(lane: &str, group: &AgentChangeGroup, path: &str) -> AgentFileTouch {
    let mut touch = agent_file_touch(lane, group);
    touch.diff_command = Some(agent_file_diff_command(lane, &touch, path));
    touch
}

fn agent_file_touch(lane: &str, group: &AgentChangeGroup) -> AgentFileTouch {
    let diff_command = if group.kind == "turn" {
        Some(agent_turn_diff_command(lane, Some(group.index), None, true))
    } else {
        group.operation_id.as_ref().map(|operation| {
            format!(
                "crabdb agent diff {lane} --operation {} --patch",
                operation.0
            )
        })
    };
    AgentFileTouch {
        kind: group.kind.clone(),
        index: group.index,
        id: group.id.clone(),
        turn_id: group.turn_id.clone(),
        operation_id: group.operation_id.clone(),
        checkpoint: group.checkpoint.clone(),
        prompt_preview: group.prompt_preview.clone(),
        diff_command,
    }
}

fn agent_compare_summary(
    left: &AgentTaskReport,
    right: &AgentTaskReport,
    left_risk: &AgentRiskReport,
    right_risk: &AgentRiskReport,
    shared_count: usize,
    left_only_count: usize,
    right_only_count: usize,
) -> String {
    if shared_count > 0 {
        let file_word = if shared_count == 1 { "file" } else { "files" };
        return format!(
            "`{}` and `{}` both changed {shared_count} {file_word}; review the overlap before applying either task.",
            agent_task_label(left),
            agent_task_label(right)
        );
    }
    if matches!(left_risk.level, AgentRiskLevel::Blocking)
        || matches!(right_risk.level, AgentRiskLevel::Blocking)
    {
        return format!(
            "The tasks do not touch the same files, but at least one has blocking risk; resolve blockers before choosing an apply order. Left-only files: {left_only_count}. Right-only files: {right_only_count}."
        );
    }
    format!(
        "The tasks do not touch the same files. Left-only files: {left_only_count}. Right-only files: {right_only_count}. Compare risk and dry-run apply plans before applying."
    )
}

fn agent_compare_recommendation(
    left: &AgentTaskReport,
    right: &AgentTaskReport,
    left_risk: &AgentRiskReport,
    right_risk: &AgentRiskReport,
    shared_paths: &[AgentComparePath],
) -> StatusSuggestion {
    if let Some(shared) = shared_paths.first() {
        return StatusSuggestion {
            command: format!("crabdb agent why {} {}", left.lane, shared.path),
            reason: "start by explaining a file changed by both tasks".to_string(),
        };
    }
    if matches!(
        left.status,
        AgentTaskStatus::Blocked | AgentTaskStatus::Conflicted | AgentTaskStatus::Dirty
    ) {
        return StatusSuggestion {
            command: format!("crabdb agent review-plan {}", left.lane),
            reason: "the left task needs attention before apply".to_string(),
        };
    }
    if matches!(
        right.status,
        AgentTaskStatus::Blocked | AgentTaskStatus::Conflicted | AgentTaskStatus::Dirty
    ) {
        return StatusSuggestion {
            command: format!("crabdb agent review-plan {}", right.lane),
            reason: "the right task needs attention before apply".to_string(),
        };
    }
    let preferred = if left_risk.score <= right_risk.score {
        left
    } else {
        right
    };
    StatusSuggestion {
        command: format!("crabdb agent land {} --dry-run", preferred.lane),
        reason: "preview applying the lower-risk task first".to_string(),
    }
}

fn agent_task_label(task: &AgentTaskReport) -> &str {
    if task.title.trim().is_empty() {
        &task.name
    } else {
        &task.title
    }
}

fn agent_shell_arg(value: &str) -> String {
    if value.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.' | '@' | ':' | '=')
    }) {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn agent_turn_diff_command(
    lane: &str,
    turn_index: Option<usize>,
    file: Option<&str>,
    patch: bool,
) -> String {
    let mut command = format!("crabdb agent turn-diff {lane}");
    if let Some(turn_index) = turn_index {
        command.push_str(&format!(" --turn {turn_index}"));
    }
    if let Some(file) = file {
        command.push_str(&format!(" --file {}", agent_shell_arg(file)));
    }
    if patch {
        command.push_str(" --patch");
    }
    command
}

fn agent_warning_risk_weight(code: &str) -> u8 {
    match code {
        "missing_latest_test" => 25,
        "missing_latest_eval" => 10,
        "stale_lane_base" => 20,
        "no_changed_paths" => 5,
        _ => 10,
    }
}

fn agent_push_risk_reason(
    reasons: &mut Vec<AgentRiskReason>,
    code: &str,
    severity: &str,
    message: &str,
) {
    if !reasons.iter().any(|reason| reason.code == code) {
        reasons.push(AgentRiskReason {
            code: code.to_string(),
            severity: severity.to_string(),
            message: message.to_string(),
        });
    }
}

fn agent_changed_manifest_or_api_surface(paths: &[FileDiffSummary]) -> bool {
    paths.iter().any(|path| {
        path.path == "Cargo.toml"
            || path.path == "Cargo.lock"
            || path.path == "package.json"
            || path.path == "package-lock.json"
            || path.path == "pnpm-lock.yaml"
            || path.path == "yarn.lock"
            || path.path.ends_with("/Cargo.toml")
            || path.path.ends_with("/package.json")
            || path.path.ends_with("/src/lib.rs")
            || path.path.ends_with("/lib.rs")
    })
}

fn agent_path_looks_like_test(path: &str) -> bool {
    path.contains("/tests/")
        || path.contains("/test/")
        || path.ends_with("_test.rs")
        || path.ends_with("_tests.rs")
        || path.ends_with(".test.ts")
        || path.ends_with(".test.tsx")
        || path.ends_with(".spec.ts")
        || path.ends_with(".spec.tsx")
        || path.ends_with(".test.js")
        || path.ends_with(".spec.js")
}

fn agent_risk_summary(level: &AgentRiskLevel, score: u8, task: &AgentTaskReport) -> String {
    let level = match level {
        AgentRiskLevel::Low => "low",
        AgentRiskLevel::Medium => "medium",
        AgentRiskLevel::High => "high",
        AgentRiskLevel::Blocking => "blocking",
    };
    let changed = agent_story_changed_phrase(&task.changed_paths);
    format!(
        "Risk is {level} ({score}/100): {} changed, status is {}.",
        changed,
        agent_story_status_phrase(&task.status)
    )
}

fn agent_task_suggestions(lane: &str, status: &AgentTaskStatus) -> Vec<StatusSuggestion> {
    match status {
        AgentTaskStatus::Empty => vec![StatusSuggestion {
            command: "crabdb agent setup --provider claude-code --editor vscode".to_string(),
            reason: "configure an editor once, then start an agent task".to_string(),
        }],
        AgentTaskStatus::Dirty => vec![StatusSuggestion {
            command: format!("crabdb agent land {lane} --dry-run"),
            reason: "preview recording and applying the dirty agent workdir".to_string(),
        }],
        AgentTaskStatus::Ready => vec![StatusSuggestion {
            command: format!("crabdb agent land {lane} --dry-run"),
            reason: "preview the safe Git apply plan".to_string(),
        }],
        AgentTaskStatus::Conflicted | AgentTaskStatus::Blocked => vec![StatusSuggestion {
            command: format!("crabdb agent view {lane}"),
            reason: "inspect blockers before applying".to_string(),
        }],
        AgentTaskStatus::Applied => vec![StatusSuggestion {
            command: format!("crabdb agent view {lane}"),
            reason: "inspect the applied transcript and checkpoint".to_string(),
        }],
        AgentTaskStatus::Active => vec![StatusSuggestion {
            command: format!("crabdb agent view {lane}"),
            reason: "inspect current task activity".to_string(),
        }],
    }
}

fn agent_next_report_from_view(
    view: &AgentTaskViewReport,
    review_progress: Option<&AgentReviewProgress>,
) -> AgentNextReport {
    let lane = &view.task.lane;
    let (focus, summary, primary) = match &view.task.status {
        AgentTaskStatus::Empty => (
            "setup",
            "No agent task is available.",
            StatusSuggestion {
                command: "crabdb agent setup --provider claude-code --editor vscode".to_string(),
                reason: "configure an editor once, then start an agent task".to_string(),
            },
        ),
        AgentTaskStatus::Active => (
            "inspect",
            "An agent task is active or has no recorded checkpoint yet.",
            StatusSuggestion {
                command: format!("crabdb agent view {lane}"),
                reason: "inspect current task activity, transcript, and captured events"
                    .to_string(),
            },
        ),
        AgentTaskStatus::Dirty => (
            "preview_apply",
            "The materialized agent workdir has unrecorded changes.",
            StatusSuggestion {
                command: format!("crabdb agent land {lane} --dry-run"),
                reason: "preview recording the workdir and applying the task without mutating Git"
                    .to_string(),
            },
        ),
        AgentTaskStatus::Ready => agent_ready_next_action(lane, review_progress),
        AgentTaskStatus::Blocked => (
            "resolve_blockers",
            "The agent task is blocked and needs review before applying.",
            StatusSuggestion {
                command: format!("crabdb agent review-plan {lane}"),
                reason: "inspect blockers and decide whether to continue, rewind, or discard"
                    .to_string(),
            },
        ),
        AgentTaskStatus::Conflicted => (
            "resolve_conflicts",
            "The agent task has conflicts and cannot be applied safely yet.",
            StatusSuggestion {
                command: format!("crabdb agent review-plan {lane}"),
                reason: "inspect conflicts before any apply attempt".to_string(),
            },
        ),
        AgentTaskStatus::Applied => (
            "inspect_applied",
            "The agent task has already been applied.",
            StatusSuggestion {
                command: format!("crabdb agent view {lane}"),
                reason: "inspect the applied transcript, tools, and checkpoint".to_string(),
            },
        ),
    };
    let suggestions = agent_next_suggestions(view, review_progress, &primary);
    AgentNextReport {
        status: view.task.status.clone(),
        task: Some(view.task.clone()),
        focus: focus.to_string(),
        summary: summary.to_string(),
        primary,
        suggestions,
    }
}

fn agent_next_suggestions(
    view: &AgentTaskViewReport,
    review_progress: Option<&AgentReviewProgress>,
    primary: &StatusSuggestion,
) -> Vec<StatusSuggestion> {
    let lane = &view.task.lane;
    let mut suggestions = Vec::new();
    let mut push = |command: String, reason: &str| {
        if command != primary.command
            && !suggestions
                .iter()
                .any(|suggestion: &StatusSuggestion| suggestion.command == command)
        {
            suggestions.push(StatusSuggestion {
                command,
                reason: reason.to_string(),
            });
        }
    };

    if view.task.status != AgentTaskStatus::Empty {
        push(
            format!("crabdb agent story {lane}"),
            "read the plain-language task summary",
        );
        push(
            format!("crabdb agent risk {lane}"),
            "check apply risk and mitigation steps",
        );
    }

    match &view.task.status {
        AgentTaskStatus::Empty => {}
        AgentTaskStatus::Active => {
            push(
                format!("crabdb agent changes {lane}"),
                "show any recorded prompt-to-checkpoint changes",
            );
        }
        AgentTaskStatus::Dirty => {
            push(
                format!("crabdb agent view {lane}"),
                "inspect the task before recording or applying dirty workdir changes",
            );
            push(
                format!("crabdb agent undo {lane}"),
                "undo the latest turn if the task went sideways",
            );
        }
        AgentTaskStatus::Ready => {
            if review_progress.is_some_and(|progress| progress.status != "up_to_date") {
                push(
                    format!("crabdb agent new {lane}"),
                    "inspect changes not yet marked reviewed",
                );
            }
            push(
                format!("crabdb agent changes {lane}"),
                "see changes grouped by prompt before applying",
            );
            push(
                agent_turn_diff_command(lane, None, None, true),
                "inspect the patch from the latest completed turn",
            );
            push(
                format!("crabdb agent land {lane} --dry-run"),
                "preview the Git apply plan without mutating Git",
            );
        }
        AgentTaskStatus::Blocked | AgentTaskStatus::Conflicted => {
            push(
                format!("crabdb agent changes {lane}"),
                "locate which turn or operation introduced the issue",
            );
            push(
                format!("crabdb agent undo {lane}"),
                "undo the latest completed turn after review",
            );
        }
        AgentTaskStatus::Applied => {
            push(
                format!("crabdb agent changes {lane}"),
                "see the applied task by prompt or operation",
            );
            push(
                "crabdb agent list".to_string(),
                "choose another agent task to inspect",
            );
        }
    }
    suggestions
}

fn agent_ready_next_action(
    lane: &str,
    review_progress: Option<&AgentReviewProgress>,
) -> (&'static str, &'static str, StatusSuggestion) {
    match review_progress {
        Some(progress) if progress.status == "up_to_date" && progress.reviewed.is_some() => (
            "preview_apply",
            "The current checkpoint has been marked reviewed and is ready for apply preview.",
            StatusSuggestion {
                command: format!("crabdb agent land {lane} --dry-run"),
                reason: "preview the safe Git apply plan after review".to_string(),
            },
        ),
        Some(progress) if progress.changed_paths > 0 => (
            "review_new",
            "The agent task has changes that have not been marked reviewed.",
            StatusSuggestion {
                command: format!("crabdb agent new {lane}"),
                reason: "inspect changes not yet marked reviewed".to_string(),
            },
        ),
        _ => (
            "review",
            "The agent task is ready for human review before applying.",
            StatusSuggestion {
                command: format!("crabdb agent review-plan {lane}"),
                reason: "review changed files, transcript, blockers, warnings, and next steps"
                    .to_string(),
            },
        ),
    }
}

fn agent_brief_tool_summaries(
    view: &AgentTaskViewReport,
    groups: &[AgentChangeGroup],
) -> Vec<String> {
    let mut tools = Vec::new();
    let mut push_tool = |tool: &String| {
        if !tools.iter().any(|existing| existing == tool) {
            tools.push(tool.clone());
        }
    };
    for group in groups {
        for tool in &group.tool_summaries {
            push_tool(tool);
        }
    }
    if let Some(transcript) = &view.transcript {
        for turn in &transcript.turns {
            for tool in &turn.tool_summaries {
                push_tool(tool);
            }
        }
    }
    tools
}

fn agent_brief_suggestions(lane: &str, next: &AgentNextReport) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.primary.clone()];
    for suggestion in &next.suggestions {
        if !suggestions
            .iter()
            .any(|existing| existing.command == suggestion.command)
        {
            suggestions.push(suggestion.clone());
        }
    }
    for suggestion in [
        StatusSuggestion {
            command: format!("crabdb agent changes {lane}"),
            reason: "inspect changes grouped by prompt or operation".to_string(),
        },
        StatusSuggestion {
            command: agent_turn_diff_command(lane, None, None, true),
            reason: "inspect the latest turn patch".to_string(),
        },
        StatusSuggestion {
            command: format!("crabdb agent land {lane} --dry-run"),
            reason: "preview the safe Git apply plan".to_string(),
        },
    ] {
        if !suggestions
            .iter()
            .any(|existing| existing.command == suggestion.command)
        {
            suggestions.push(suggestion);
        }
    }
    suggestions
}

fn agent_editor_snippet(editor: &str, provider: &str, command: &[String]) -> String {
    match editor {
        "vscode" => serde_json::to_string_pretty(&serde_json::json!({
            (format!("CrabDB {}", provider_display_name(provider))): {
                "command": command.first().cloned().unwrap_or_default(),
                "args": command.iter().skip(1).cloned().collect::<Vec<_>>(),
                "env": {}
            }
        }))
        .unwrap_or_else(|_| "{}".to_string()),
        "zed" => serde_json::to_string_pretty(&serde_json::json!({
            "agent_servers": {
                (format!("crabdb-{}", provider)): {
                    "type": "custom",
                    "command": command.first().cloned().unwrap_or_default(),
                    "args": command.iter().skip(1).cloned().collect::<Vec<_>>()
                }
            }
        }))
        .unwrap_or_else(|_| "{}".to_string()),
        _ => format!("ACP command:\n{}", shell_join(command)),
    }
}

fn provider_display_name(provider: &str) -> String {
    match provider {
        "claude-code" | "claude" => "Claude Code".to_string(),
        other => other.to_string(),
    }
}

fn sanitize_agent_ref_component(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('-');
        }
    }
    let out = out
        .trim_matches(|ch| matches!(ch, '-' | '_' | '.'))
        .chars()
        .take(48)
        .collect::<String>();
    if out.is_empty() {
        "task".to_string()
    } else {
        out
    }
}

fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| {
            if part.chars().all(|ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.' | '@' | ':')
            }) {
                part.clone()
            } else {
                shell_quote(part)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn default_agent_apply_message(lane: &str) -> String {
    format!("Apply agent task {lane}")
}

fn default_agent_apply_message_for_task(task: &AgentTaskReport) -> String {
    let title = task.title.trim();
    if title.is_empty() || title == task.name {
        default_agent_apply_message(&task.lane)
    } else {
        format!("Apply agent task: {}", single_line_preview(title, 72))
    }
}
