use super::*;
use crate::db::util::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const AGENT_REVIEWED_EVENT: &str = "agent_reviewed";
const AGENT_FILE_REVIEWED_EVENT: &str = "agent_file_reviewed";
const AGENT_TASK_ARCHIVED_EVENT: &str = "agent_task_archived";
const AGENT_TASK_UNARCHIVED_EVENT: &str = "agent_task_unarchived";

struct AgentReviewProgress {
    status: String,
    reviewed: Option<AgentReviewMarker>,
    changed_paths: usize,
    changed_lines: u64,
}

#[derive(Clone, Debug)]
enum AgentAskRoute {
    Inbox,
    Board,
    Stack,
    Next,
    Guide,
    Dashboard,
    ReviewData,
    Actions,
    Summary,
    Validate,
    TestPlan,
    Brief,
    Story,
    Risk,
    Impact,
    ReviewMap,
    Confidence,
    Ready,
    Diagnose,
    Receipt,
    Handoff,
    Pr,
    Changes,
    ChangesByFile,
    Tools,
    TaskDiff { file: Option<String>, patch: bool },
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
    ReviewFlow,
    Review,
    Focus,
    View,
}

impl AgentAskRoute {
    fn intent(&self) -> &'static str {
        match self {
            AgentAskRoute::Inbox => "inbox",
            AgentAskRoute::Board => "board",
            AgentAskRoute::Stack => "stack",
            AgentAskRoute::Next => "next",
            AgentAskRoute::Guide => "guide",
            AgentAskRoute::Dashboard => "dashboard",
            AgentAskRoute::ReviewData => "review_data",
            AgentAskRoute::Actions => "actions",
            AgentAskRoute::Summary => "summary",
            AgentAskRoute::Validate => "validate",
            AgentAskRoute::TestPlan => "test_plan",
            AgentAskRoute::Brief => "brief",
            AgentAskRoute::Story => "story",
            AgentAskRoute::Risk => "risk",
            AgentAskRoute::Impact => "impact",
            AgentAskRoute::ReviewMap => "review_map",
            AgentAskRoute::Confidence => "confidence",
            AgentAskRoute::Ready => "ready",
            AgentAskRoute::Diagnose => "diagnose",
            AgentAskRoute::Receipt => "receipt",
            AgentAskRoute::Handoff => "handoff",
            AgentAskRoute::Pr => "pr",
            AgentAskRoute::Changes => "changes",
            AgentAskRoute::ChangesByFile => "changes",
            AgentAskRoute::Tools => "tools",
            AgentAskRoute::TaskDiff { .. } => "diff",
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
            AgentAskRoute::ReviewFlow => "review_flow",
            AgentAskRoute::Review => "review",
            AgentAskRoute::Focus => "focus",
            AgentAskRoute::View => "view",
        }
    }

    fn tool(&self) -> &'static str {
        match self {
            AgentAskRoute::Inbox => "crabdb.agent_inbox",
            AgentAskRoute::Board => "crabdb.agent_board",
            AgentAskRoute::Stack => "crabdb.agent_stack",
            AgentAskRoute::Next => "crabdb.agent_next",
            AgentAskRoute::Guide => "crabdb.agent_guide",
            AgentAskRoute::Dashboard => "crabdb.agent_dashboard",
            AgentAskRoute::ReviewData => "crabdb.agent_review_data",
            AgentAskRoute::Actions => "crabdb.agent_review_data",
            AgentAskRoute::Summary => "crabdb.agent_summary",
            AgentAskRoute::Validate => "crabdb.agent_validate",
            AgentAskRoute::TestPlan => "crabdb.agent_test_plan",
            AgentAskRoute::Brief => "crabdb.agent_brief",
            AgentAskRoute::Story => "crabdb.agent_story",
            AgentAskRoute::Risk => "crabdb.agent_risk",
            AgentAskRoute::Impact => "crabdb.agent_impact",
            AgentAskRoute::ReviewMap => "crabdb.agent_review_map",
            AgentAskRoute::Confidence => "crabdb.agent_confidence",
            AgentAskRoute::Ready => "crabdb.agent_ready",
            AgentAskRoute::Diagnose => "crabdb.agent_diagnose",
            AgentAskRoute::Receipt => "crabdb.agent_receipt",
            AgentAskRoute::Handoff => "crabdb.agent_handoff",
            AgentAskRoute::Pr => "crabdb.agent_pr",
            AgentAskRoute::Changes => "crabdb.agent_changes",
            AgentAskRoute::ChangesByFile => "crabdb.agent_changes",
            AgentAskRoute::Tools => "crabdb.agent_tools",
            AgentAskRoute::TaskDiff { .. } => "crabdb.agent_diff",
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
            AgentAskRoute::ReviewFlow => "crabdb.agent_review_flow",
            AgentAskRoute::Review => "crabdb.agent_review",
            AgentAskRoute::Focus => "crabdb.agent_focus",
            AgentAskRoute::View => "crabdb.agent_view",
        }
    }

    fn cli_command(&self, selector: &str) -> String {
        let selector = agent_shell_arg(selector);
        match self {
            AgentAskRoute::Inbox => "crabdb agent inbox".to_string(),
            AgentAskRoute::Board => "crabdb agent board".to_string(),
            AgentAskRoute::Stack => "crabdb agent stack".to_string(),
            AgentAskRoute::Next => format!("crabdb agent next {selector}"),
            AgentAskRoute::Guide => format!("crabdb agent guide {selector}"),
            AgentAskRoute::Dashboard => format!("crabdb agent dashboard {selector}"),
            AgentAskRoute::ReviewData => format!("crabdb agent review-data {selector}"),
            AgentAskRoute::Actions => format!("crabdb agent action {selector}"),
            AgentAskRoute::Summary => format!("crabdb agent summary {selector}"),
            AgentAskRoute::Validate => format!("crabdb agent validate {selector}"),
            AgentAskRoute::TestPlan => format!("crabdb agent test-plan {selector}"),
            AgentAskRoute::Brief => format!("crabdb agent brief {selector}"),
            AgentAskRoute::Story => format!("crabdb agent story {selector}"),
            AgentAskRoute::Risk => format!("crabdb agent risk {selector}"),
            AgentAskRoute::Impact => format!("crabdb agent impact {selector}"),
            AgentAskRoute::ReviewMap => format!("crabdb agent review-map {selector}"),
            AgentAskRoute::Confidence => format!("crabdb agent confidence {selector}"),
            AgentAskRoute::Ready => format!("crabdb agent can-land {selector}"),
            AgentAskRoute::Diagnose => format!("crabdb agent recover {selector}"),
            AgentAskRoute::Receipt => format!("crabdb agent receipt {selector}"),
            AgentAskRoute::Handoff => format!("crabdb agent handoff {selector}"),
            AgentAskRoute::Pr => format!("crabdb agent pr {selector}"),
            AgentAskRoute::Changes => format!("crabdb agent changes {selector}"),
            AgentAskRoute::ChangesByFile => format!("crabdb agent changes {selector} --by-file"),
            AgentAskRoute::Tools => format!("crabdb agent tools {selector}"),
            AgentAskRoute::TaskDiff { file, patch } => {
                let mut command = format!("crabdb agent diff {selector}");
                if let Some(path) = file {
                    command.push_str(&format!(" --file {}", agent_shell_arg(path)));
                }
                if *patch {
                    command.push_str(" --patch");
                }
                command
            }
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
            AgentAskRoute::ReviewFlow => format!("crabdb agent review-flow {selector}"),
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
            suggestions: vec![
                StatusSuggestion {
                    command: format!("crabdb agent doctor --provider {provider}"),
                    reason: "verify the workspace and provider before the first editor session"
                        .to_string(),
                },
                StatusSuggestion {
                    command: "crabdb agent".to_string(),
                    reason: "after one prompt, show the task inbox and next useful action"
                        .to_string(),
                },
                StatusSuggestion {
                    command: "crabdb agent action".to_string(),
                    reason: "show runnable setup, review, validation, apply, and recovery actions"
                        .to_string(),
                },
            ],
        })
    }

    pub fn list_agent_tasks(&self) -> Result<AgentTaskListReport> {
        self.list_agent_tasks_with_options(false)
    }

    pub fn list_agent_tasks_with_options(
        &self,
        include_archived: bool,
    ) -> Result<AgentTaskListReport> {
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
            let task = self.agent_task_for_lane_details(lane, 10)?;
            if !include_archived && task.archived {
                continue;
            }
            tasks.push(task);
        }
        Ok(AgentTaskListReport {
            include_archived,
            tasks,
        })
    }

    pub fn agent_inbox(&self) -> Result<AgentInboxReport> {
        self.agent_inbox_with_options(false)
    }

    pub fn agent_board(&self) -> Result<AgentBoardReport> {
        self.agent_board_with_options(false)
    }

    pub fn agent_stack(&self) -> Result<AgentStackReport> {
        self.agent_stack_with_options(false)
    }

    pub fn agent_board_with_options(&self, include_archived: bool) -> Result<AgentBoardReport> {
        let inbox = self.agent_inbox_with_options(include_archived)?;
        let items = inbox
            .items
            .iter()
            .map(agent_board_item_from_inbox)
            .collect::<Vec<_>>();
        let ready_count = items
            .iter()
            .filter(|item| {
                !item.task.archived && matches!(item.task.status, AgentTaskStatus::Ready)
            })
            .count();
        let active_count = items
            .iter()
            .filter(|item| {
                !item.task.archived && matches!(item.task.status, AgentTaskStatus::Active)
            })
            .count();
        let blocked_count = items
            .iter()
            .filter(|item| {
                !item.task.archived && matches!(item.task.status, AgentTaskStatus::Blocked)
            })
            .count();
        let conflicted_count = items
            .iter()
            .filter(|item| {
                !item.task.archived && matches!(item.task.status, AgentTaskStatus::Conflicted)
            })
            .count();
        let applied_count = items
            .iter()
            .filter(|item| {
                !item.task.archived && matches!(item.task.status, AgentTaskStatus::Applied)
            })
            .count();
        let mut suggestions = vec![inbox.next.clone()];
        agent_push_suggestion(
            &mut suggestions,
            "crabdb agent inbox".to_string(),
            "open the detailed task queue with review-first metadata",
        );
        agent_push_suggestion(
            &mut suggestions,
            "crabdb agent guide".to_string(),
            "show the shortest state-aware workflow for the current task",
        );
        if inbox.archived_count > 0 && !include_archived {
            agent_push_suggestion(
                &mut suggestions,
                "crabdb agent board --all".to_string(),
                "include archived tasks on the board",
            );
        }
        if inbox.total == 0 {
            agent_push_suggestion(
                &mut suggestions,
                "crabdb agent start --provider claude-code".to_string(),
                "start a terminal agent task without configuring an editor",
            );
        }

        Ok(AgentBoardReport {
            include_archived,
            total: inbox.total,
            attention_count: inbox.attention_count,
            ready_count,
            active_count,
            blocked_count,
            conflicted_count,
            applied_count,
            archived_count: inbox.archived_count,
            columns: agent_board_columns(&items),
            items,
            next: inbox.next,
            suggestions,
        })
    }

    pub fn agent_stack_with_options(&self, include_archived: bool) -> Result<AgentStackReport> {
        let tasks = self.list_agent_tasks_with_options(include_archived)?.tasks;
        if tasks.is_empty() {
            let next = StatusSuggestion {
                command: "crabdb agent setup".to_string(),
                reason: "configure an editor once, then start an agent task".to_string(),
            };
            return Ok(AgentStackReport {
                include_archived,
                total: 0,
                ready_count: 0,
                blocked_count: 0,
                overlap_count: 0,
                summary: "No agent tasks are recorded yet.".to_string(),
                shared_paths: Vec::new(),
                items: Vec::new(),
                apply_order: Vec::new(),
                suggestions: vec![
                    next.clone(),
                    StatusSuggestion {
                        command: "crabdb agent start --provider claude-code".to_string(),
                        reason: "start a terminal agent task without configuring an editor"
                            .to_string(),
                    },
                ],
                next,
            });
        }

        let shared_paths = agent_stack_shared_paths(&tasks);
        let shared_by_lane = agent_stack_shared_paths_by_lane(&shared_paths);
        let mut items = Vec::new();
        for task in &tasks {
            let view = self.agent_task_view(&task.lane)?;
            let risk = agent_risk_report_from_view(&view);
            let task_shared_paths = shared_by_lane.get(&task.lane).cloned().unwrap_or_default();
            let status = agent_stack_item_status(&view.task, &risk, !task_shared_paths.is_empty());
            let applyable = agent_stack_item_applyable(&view.task, &risk, &task_shared_paths);
            let next = agent_stack_item_next(&view.task, &status);
            items.push(AgentStackItem {
                rank: 0,
                task: view.task,
                risk,
                status,
                shared_paths: task_shared_paths,
                applyable,
                next,
            });
        }
        items.sort_by_key(agent_stack_item_sort_key);
        for (idx, item) in items.iter_mut().enumerate() {
            item.rank = idx + 1;
        }

        let apply_order = items
            .iter()
            .filter(|item| item.applyable)
            .map(|item| item.task.lane.clone())
            .collect::<Vec<_>>();
        let ready_count = items
            .iter()
            .filter(|item| {
                !item.task.archived
                    && matches!(
                        item.task.status,
                        AgentTaskStatus::Ready | AgentTaskStatus::Dirty
                    )
            })
            .count();
        let blocked_count = items
            .iter()
            .filter(|item| {
                !item.task.archived
                    && matches!(
                        item.task.status,
                        AgentTaskStatus::Blocked | AgentTaskStatus::Conflicted
                    )
            })
            .count();
        let overlap_count = shared_paths.len();
        let next = agent_stack_next(&items, &shared_paths);
        let mut suggestions = vec![next.clone()];
        agent_push_suggestion(
            &mut suggestions,
            "crabdb agent board".to_string(),
            "return to the grouped task board",
        );
        agent_push_suggestion(
            &mut suggestions,
            "crabdb agent inbox".to_string(),
            "open the detailed task inbox with review-first metadata",
        );
        if !include_archived {
            agent_push_suggestion(
                &mut suggestions,
                "crabdb agent stack --all".to_string(),
                "include archived tasks in the apply-order view",
            );
        }
        let summary = agent_stack_summary(tasks.len(), ready_count, blocked_count, overlap_count);
        Ok(AgentStackReport {
            include_archived,
            total: tasks.len(),
            ready_count,
            blocked_count,
            overlap_count,
            summary,
            shared_paths,
            items,
            apply_order,
            next,
            suggestions,
        })
    }

    pub fn agent_inbox_with_options(&self, include_archived: bool) -> Result<AgentInboxReport> {
        let tasks = self.list_agent_tasks_with_options(include_archived)?.tasks;
        if tasks.is_empty() {
            let next = StatusSuggestion {
                command: "crabdb agent setup".to_string(),
                reason: "configure an editor once, then start an agent task".to_string(),
            };
            return Ok(AgentInboxReport {
                include_archived,
                total: 0,
                attention_count: 0,
                archived_count: 0,
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
        let attention_count = items
            .iter()
            .filter(|item| {
                !item.task.archived && !matches!(item.task.status, AgentTaskStatus::Applied)
            })
            .count();
        let archived_count = items.iter().filter(|item| item.task.archived).count();
        let next = items
            .iter()
            .find(|item| {
                !item.task.archived && !matches!(item.task.status, AgentTaskStatus::Applied)
            })
            .map(|item| item.next.clone())
            .or_else(|| {
                items
                    .iter()
                    .find(|item| !item.task.archived)
                    .map(|item| item.next.clone())
            })
            .or_else(|| items.first().map(|item| item.next.clone()))
            .unwrap_or_else(|| StatusSuggestion {
                command: "crabdb agent setup".to_string(),
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
        if archived_count == 0 {
            agent_push_suggestion(
                &mut suggestions,
                "crabdb agent inbox --all".to_string(),
                "show archived tasks as well as active tasks",
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
            include_archived,
            total: tasks.len(),
            attention_count,
            archived_count,
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
                    command: "crabdb agent setup".to_string(),
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
                command: "crabdb agent setup".to_string(),
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

    pub fn agent_guide(&self, selector: &str) -> Result<AgentGuideReport> {
        let next = self.agent_next(selector)?;
        let task = next.task.clone();
        let headline = agent_guide_headline(task.as_ref(), &next.status);
        let current_state = agent_guide_current_state(task.as_ref(), &next);
        let steps = agent_guide_steps(task.as_ref(), &next.primary);
        let concepts = agent_guide_concepts();
        let mut suggestions = vec![next.primary.clone()];
        for step in &steps {
            agent_push_suggestion(&mut suggestions, step.command.clone(), &step.reason);
        }
        agent_push_suggestion(
            &mut suggestions,
            "crabdb agent ask what should I do next".to_string(),
            "use plain language when you do not remember a specific command",
        );
        Ok(AgentGuideReport {
            status: next.status,
            selector: selector.to_string(),
            task,
            headline,
            current_state,
            primary: next.primary,
            steps,
            concepts,
            suggestions,
        })
    }

    pub fn agent_dashboard(&mut self, selector: &str) -> Result<AgentDashboardReport> {
        let next = self.agent_next(selector)?;
        let Some(task) = next.task.clone() else {
            let mut suggestions = vec![next.primary.clone()];
            suggestions.extend(next.suggestions.clone());
            return Ok(AgentDashboardReport {
                status: next.status,
                task: None,
                summary: next.summary,
                next: next.primary,
                ready: None,
                validation: None,
                focus: None,
                changes: None,
                suggestions,
            });
        };
        let lane = task.lane.clone();
        let ready = self.agent_ready(&lane)?;
        let validation = self.agent_validate(&lane)?;
        let changes = self.agent_changes_with_options(&lane, false, true)?;
        let focus = if task.changed_paths.is_empty() {
            None
        } else {
            Some(self.agent_focus(&lane, None, false)?)
        };
        let summary = agent_dashboard_summary(&task, &ready, &validation, focus.as_ref());
        let mut suggestions = vec![next.primary.clone()];
        if let Some(focus) = &focus {
            if let Some(command) = &focus.open_command {
                agent_push_suggestion(
                    &mut suggestions,
                    command.clone(),
                    "open the highest-priority file in your configured editor",
                );
            }
            agent_push_suggestion(
                &mut suggestions,
                format!("crabdb agent focus {lane}"),
                "inspect why this file is the next review target",
            );
        }
        agent_push_suggestion(
            &mut suggestions,
            validation.next.command.clone(),
            &validation.next.reason,
        );
        agent_push_suggestion(
            &mut suggestions,
            ready.next.command.clone(),
            &ready.next.reason,
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent confidence {lane}"),
            "show one go/no-go verdict across review, validation, risk, and apply preflight",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent changes {lane} --by-file"),
            "review every changed file as its own card",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent view {lane}"),
            "open the full transcript, tools, changed paths, and checkpoint",
        );
        Ok(AgentDashboardReport {
            status: task.status.clone(),
            task: Some(task),
            summary,
            next: next.primary,
            ready: Some(ready),
            validation: Some(validation),
            focus,
            changes: Some(changes),
            suggestions,
        })
    }

    pub fn agent_review_data(&mut self, selector: &str) -> Result<AgentReviewDataReport> {
        let review_map = self.agent_review_map(selector)?;
        let lane = review_map.task.lane.clone();
        let confidence = self.agent_confidence(&lane)?;
        let changes_by_file = self.agent_changes_with_options(&lane, false, true)?;
        let files = self.agent_files(&lane)?;
        let focus = if review_map.changed_paths.is_empty() {
            None
        } else {
            Some(self.agent_focus(&lane, None, false)?)
        };
        let total_files = review_map
            .areas
            .iter()
            .map(|area| area.files.len())
            .sum::<usize>();
        let reviewed_files = review_map
            .areas
            .iter()
            .flat_map(|area| area.files.iter())
            .filter(|file| file.state == "reviewed")
            .count();
        let needs_review_files = total_files.saturating_sub(reviewed_files);
        let next = confidence.next.clone();
        let summary = agent_review_data_summary(
            &review_map.task,
            &confidence,
            total_files,
            reviewed_files,
            needs_review_files,
        );
        let actions =
            agent_review_data_actions(&lane, focus.as_ref(), &confidence, needs_review_files);
        let suggestions =
            agent_review_data_suggestions(&lane, &next, focus.as_ref(), needs_review_files);
        Ok(AgentReviewDataReport {
            task: review_map.task.clone(),
            summary,
            next,
            review_status: review_map.review_status.clone(),
            ready_to_apply: confidence.ready.ready,
            readiness_status: confidence.ready.readiness_status.clone(),
            confidence_verdict: confidence.verdict.clone(),
            confidence_score: confidence.score,
            risk_level: confidence.risk.level.clone(),
            validation_status: confidence.validation.status.clone(),
            total_files,
            reviewed_files,
            needs_review_files,
            focus,
            review_map,
            changes_by_file,
            files,
            confidence,
            actions,
            suggestions,
        })
    }

    pub fn agent_ask(&mut self, selector: &str, question: &str) -> Result<AgentAskReport> {
        let route = agent_ask_route(question)?;
        let report = match &route {
            AgentAskRoute::Inbox => serde_json::to_value(self.agent_inbox()?)?,
            AgentAskRoute::Board => serde_json::to_value(self.agent_board()?)?,
            AgentAskRoute::Stack => serde_json::to_value(self.agent_stack()?)?,
            AgentAskRoute::Next => serde_json::to_value(self.agent_next(selector)?)?,
            AgentAskRoute::Guide => serde_json::to_value(self.agent_guide(selector)?)?,
            AgentAskRoute::Dashboard => serde_json::to_value(self.agent_dashboard(selector)?)?,
            AgentAskRoute::ReviewData => serde_json::to_value(self.agent_review_data(selector)?)?,
            AgentAskRoute::Actions => match self.agent_review_data(selector) {
                Ok(report) => serde_json::to_value(report)?,
                Err(Error::InvalidInput(message)) if message.contains("no agent tasks") => {
                    agent_empty_action_palette_value()
                }
                Err(err) => return Err(err),
            },
            AgentAskRoute::Summary => serde_json::to_value(self.agent_summary(selector)?)?,
            AgentAskRoute::Validate => serde_json::to_value(self.agent_validate(selector)?)?,
            AgentAskRoute::TestPlan => serde_json::to_value(self.agent_test_plan(selector)?)?,
            AgentAskRoute::Brief => serde_json::to_value(self.agent_brief(selector)?)?,
            AgentAskRoute::Story => serde_json::to_value(self.agent_story(selector)?)?,
            AgentAskRoute::Risk => serde_json::to_value(self.agent_risk(selector)?)?,
            AgentAskRoute::Impact => serde_json::to_value(self.agent_impact(selector)?)?,
            AgentAskRoute::ReviewMap => serde_json::to_value(self.agent_review_map(selector)?)?,
            AgentAskRoute::Confidence => serde_json::to_value(self.agent_confidence(selector)?)?,
            AgentAskRoute::Ready => serde_json::to_value(self.agent_ready(selector)?)?,
            AgentAskRoute::Diagnose => serde_json::to_value(self.agent_diagnose(selector)?)?,
            AgentAskRoute::Receipt => serde_json::to_value(self.agent_receipt(selector)?)?,
            AgentAskRoute::Handoff => serde_json::to_value(self.agent_handoff(selector)?)?,
            AgentAskRoute::Pr => serde_json::to_value(self.agent_pr_draft(selector)?)?,
            AgentAskRoute::Changes => serde_json::to_value(self.agent_changes(selector, false)?)?,
            AgentAskRoute::ChangesByFile => {
                serde_json::to_value(self.agent_changes_with_options(selector, false, true)?)?
            }
            AgentAskRoute::Tools => serde_json::to_value(self.agent_tools(selector)?)?,
            AgentAskRoute::TaskDiff { file, patch } => serde_json::to_value(self.agent_diff(
                selector,
                None,
                None,
                None,
                false,
                file.as_deref(),
                *patch,
            )?)?,
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
            AgentAskRoute::ReviewFlow => serde_json::to_value(self.agent_review_flow(selector)?)?,
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

    pub fn agent_tools(&self, selector: &str) -> Result<AgentToolsReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let mut groups = self.agent_turn_change_groups(&view)?;
        if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
        }
        let group_by_turn = groups
            .iter()
            .filter_map(|group| {
                group
                    .turn_id
                    .as_ref()
                    .map(|turn_id| (turn_id.clone(), group.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let available_commands = self.agent_available_commands(&view)?;
        let tools = agent_tool_entries(&lane, &view, &group_by_turn);
        let total_tool_events = tools.iter().map(|tool| tool.event_count).sum();
        let turns_with_tools = tools
            .iter()
            .flat_map(|tool| tool.turns.iter().map(|turn| turn.turn_id.as_str()))
            .collect::<BTreeSet<_>>()
            .len();
        let summary = agent_tools_summary(
            &view.task,
            total_tool_events,
            tools.len(),
            turns_with_tools,
            available_commands.len(),
        );
        let suggestions = agent_tools_suggestions(&lane, &tools);
        Ok(AgentToolsReport {
            task: view.task,
            lane,
            summary,
            total_tool_events,
            unique_tools: tools.len(),
            turns_with_tools,
            available_commands,
            tools,
            suggestions,
        })
    }

    pub fn agent_risk(&self, selector: &str) -> Result<AgentRiskReport> {
        let view = self.agent_task_view(selector)?;
        Ok(agent_risk_report_from_view(&view))
    }

    pub fn agent_impact(&self, selector: &str) -> Result<AgentImpactReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let changed_paths = view.task.changed_paths.clone();
        let changed_lines = changed_paths
            .iter()
            .map(|path| path.additions + path.deletions)
            .sum();
        let mut areas = agent_impact_areas(&lane, &changed_paths);
        let highest_impact = areas
            .iter()
            .map(|area| area.severity.as_str())
            .max_by_key(|severity| agent_impact_severity_rank(severity))
            .unwrap_or("none")
            .to_string();
        areas.sort_by(|left, right| {
            agent_impact_severity_rank(&right.severity)
                .cmp(&agent_impact_severity_rank(&left.severity))
                .then_with(|| right.changed_lines.cmp(&left.changed_lines))
                .then_with(|| left.label.cmp(&right.label))
        });
        let risk = agent_risk_report_from_view(&view);
        let validation = self.agent_validate(&lane)?;
        let recommendations =
            agent_impact_recommendations(&lane, &areas, &risk, &validation, &changed_paths);
        let suggestions = agent_impact_suggestions(&lane, &recommendations);
        let summary = agent_impact_summary(
            &view.task,
            &areas,
            changed_lines,
            &highest_impact,
            &validation,
        );
        Ok(AgentImpactReport {
            task: view.task,
            summary,
            changed_paths,
            changed_lines,
            highest_impact,
            areas,
            risk,
            validation,
            recommendations,
            suggestions,
        })
    }

    pub fn agent_review_map(&self, selector: &str) -> Result<AgentReviewMapReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let changed_paths = view.task.changed_paths.clone();
        let progress = self.agent_review_progress_for_view(&view)?;
        let mut groups = self.agent_turn_change_groups(&view)?;
        if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
        }
        let priorities = agent_review_priorities(&lane, &changed_paths, &groups);
        let reviewed_files =
            self.valid_agent_file_review_markers(&lane, &view.task.latest_checkpoint)?;
        let mut areas = agent_review_map_areas(
            &lane,
            view.task.workdir.as_deref(),
            &changed_paths,
            &priorities,
            &progress.status,
            &reviewed_files,
        );
        let changed_lines = changed_paths
            .iter()
            .map(|path| path.additions + path.deletions)
            .sum();
        let highest_impact = areas
            .iter()
            .map(|area| area.severity.as_str())
            .max_by_key(|severity| agent_impact_severity_rank(severity))
            .unwrap_or("none")
            .to_string();
        areas.sort_by(|left, right| {
            agent_impact_severity_rank(&right.severity)
                .cmp(&agent_impact_severity_rank(&left.severity))
                .then_with(|| {
                    let right_score = right.files.first().map(|file| file.score).unwrap_or(0);
                    let left_score = left.files.first().map(|file| file.score).unwrap_or(0);
                    right_score.cmp(&left_score)
                })
                .then_with(|| right.changed_lines.cmp(&left.changed_lines))
                .then_with(|| left.label.cmp(&right.label))
        });
        let risk = agent_risk_report_from_view(&view);
        let validation = self.agent_validate(&lane)?;
        let next = agent_review_map_next(&lane, &areas, &progress, &validation);
        let suggestions = agent_review_map_suggestions(&lane, &next, &areas, &validation);
        let summary =
            agent_review_map_summary(&view.task, &areas, changed_lines, &progress, &validation);
        Ok(AgentReviewMapReport {
            task: view.task,
            summary,
            review_status: progress.status,
            reviewed: progress.reviewed,
            changed_paths,
            changed_lines,
            highest_impact,
            areas,
            risk,
            validation,
            next,
            suggestions,
        })
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
            default_apply_message: default_agent_apply_message_for_task(&view.task),
            apply_preview,
            apply_error,
            next,
            suggestions,
        })
    }

    pub fn agent_confidence(&mut self, selector: &str) -> Result<AgentConfidenceReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let progress = self.agent_review_progress_for_view(&view)?;
        let validation = self.agent_validate(&lane)?;
        let ready = self.agent_ready(&lane)?;
        let risk = ready.risk.clone();
        let factors = agent_confidence_factors(&lane, &progress, &validation, &ready, &risk);
        let score = agent_confidence_score(&factors, &risk);
        let verdict =
            agent_confidence_verdict(&view.task, &progress, &validation, &ready, &risk, score);
        let next = agent_confidence_next(&lane, &verdict, &progress, &validation, &ready);
        let suggestions = agent_confidence_suggestions(&lane, &next, &verdict, &validation, &ready);
        let summary =
            agent_confidence_summary(&view.task, &verdict, score, &progress, &validation, &ready);
        Ok(AgentConfidenceReport {
            task: view.task,
            verdict,
            score,
            summary,
            review_status: progress.status,
            reviewed: progress.reviewed,
            ready,
            validation,
            risk,
            factors,
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

    pub fn agent_handoff(&self, selector: &str) -> Result<AgentHandoffReport> {
        let report = self.agent_report(selector)?;
        let validation = agent_receipt_validation(&report.review);
        let lane = report.task.lane.clone();
        let markdown = agent_handoff_markdown(&report, &validation);
        let mut suggestions = vec![report.next.clone()];
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent focus {lane}"),
            "start the receiving reviewer at the highest-priority file",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent ready {lane}"),
            "check whether this task can be safely applied",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent receipt {lane}"),
            "print the shorter after-action receipt",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent report {lane} --markdown"),
            "print the deeper review report backing this handoff",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent close {lane}"),
            "archive the task after the receiving human or agent no longer needs it",
        );
        Ok(AgentHandoffReport {
            task: report.task.clone(),
            summary: agent_handoff_summary(&report),
            ready_to_apply: report.ready_to_apply,
            readiness_status: report.readiness_status.clone(),
            risk: report.risk.clone(),
            changed_paths: report.task.changed_paths.clone(),
            turns: report.story.turn_summaries.clone(),
            tool_summaries: report.story.tool_summaries.clone(),
            validation,
            latest_checkpoint: report.task.latest_checkpoint.clone(),
            transcript_turns: report
                .transcript
                .as_ref()
                .map(|transcript| transcript.turns.len())
                .unwrap_or(report.task.turns),
            tool_events: report.task.tool_events,
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

    pub fn agent_test_plan(&self, selector: &str) -> Result<AgentTestPlanReport> {
        let validation = self.agent_validate(selector)?;
        let impact = self.agent_impact(selector)?;
        let lane = validation.task.lane.clone();
        let steps = agent_test_plan_steps(&lane, self.workspace_root(), &validation, &impact);
        let status = agent_test_plan_status(&steps);
        let next = agent_test_plan_next(&lane, &steps);
        let suggestions = agent_test_plan_suggestions(&lane, &next, &validation, &impact);
        let summary = agent_test_plan_summary(&validation.task, &status, &steps, &impact);
        Ok(AgentTestPlanReport {
            task: validation.task.clone(),
            status,
            summary,
            validation,
            impact_areas: impact.areas,
            risk: impact.risk,
            steps,
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
        let summary = agent_focus_summary(&review, &path, &source, priority.as_ref(), &why);
        let open_path = review
            .task
            .workdir
            .as_ref()
            .map(|workdir| Path::new(workdir).join(&path).to_string_lossy().to_string());
        let open_command = open_path
            .as_ref()
            .map(|path| format!("${{EDITOR:-vi}} {}", shell_quote(path)));
        let mut suggestions = agent_focus_suggestions(&lane, &path, &next, &review);
        if let Some(command) = &open_command {
            agent_push_suggestion(
                &mut suggestions,
                command.clone(),
                "open the focused file in your configured editor",
            );
        }
        Ok(AgentFocusReport {
            task: review.task,
            path,
            open_path,
            open_command,
            source,
            summary,
            priority,
            why,
            diff,
            next,
            suggestions,
        })
    }

    pub fn agent_review_flow(&mut self, selector: &str) -> Result<AgentReviewFlowReport> {
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let progress = self.agent_review_progress_for_view(&view)?;
        let review = self.agent_review(&lane)?;
        let focus = if review.priorities.is_empty() {
            None
        } else {
            Some(self.agent_focus(&lane, None, false)?)
        };
        let new = self.agent_new(&lane, None, false)?;
        let validation = self.agent_validate(&lane)?;
        let ready = self.agent_ready(&lane)?;
        let steps = agent_review_flow_steps(&lane, &new, &validation, &ready, focus.as_ref());
        let next = steps
            .iter()
            .find(|step| step.state == "current")
            .map(|step| StatusSuggestion {
                command: step.command.clone(),
                reason: step.reason.clone(),
            })
            .unwrap_or_else(|| ready.next.clone());
        let suggestions = agent_review_flow_suggestions(
            &lane,
            &next,
            &review,
            &new,
            &validation,
            &ready,
            focus.as_ref(),
        );
        let summary =
            agent_review_flow_summary(&view.task, &progress, &validation, &ready, focus.as_ref());
        Ok(AgentReviewFlowReport {
            task: view.task.clone(),
            status: view.task.status.clone(),
            summary,
            review_status: progress.status,
            reviewed: progress.reviewed,
            new_changed_paths: progress.changed_paths,
            new_changed_lines: progress.changed_lines,
            review,
            focus,
            new,
            validation,
            ready,
            steps,
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
        self.agent_changes_with_options(selector, by_operation, false)
    }

    pub fn agent_changes_with_options(
        &self,
        selector: &str,
        by_operation: bool,
        by_file: bool,
    ) -> Result<AgentChangesReport> {
        if by_operation && by_file {
            return Err(Error::InvalidInput(
                "agent changes accepts only one grouping lens: --by-operation or --by-file"
                    .to_string(),
            ));
        }
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let branch = self.lane_branch(&lane)?;
        let mut groups = if by_operation {
            self.agent_operation_change_groups(&view)?
        } else {
            self.agent_turn_change_groups(&view)?
        };
        let detail_grouping = if by_operation {
            "operation"
        } else if groups.is_empty() {
            groups = self.agent_operation_change_groups(&view)?;
            "operation"
        } else {
            "turn"
        };
        let grouping = if by_file { "file" } else { detail_grouping };
        let cards = if by_file {
            agent_file_change_cards(&lane, &view.task.changed_paths, &groups)
        } else {
            agent_change_cards(&lane, &view.task.changed_paths, &groups)
        };
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

    pub fn agent_mark_file_reviewed(
        &mut self,
        selector: &str,
        path: &str,
        note: Option<String>,
    ) -> Result<AgentMarkFileReviewedReport> {
        let _lock = self.acquire_write_lock()?;
        let view = self.agent_task_view(selector)?;
        let lane = view.task.lane.clone();
        let branch = self.lane_branch(&lane)?;
        let change = view
            .task
            .changed_paths
            .iter()
            .find(|change| agent_file_matches_path(change, path))
            .cloned()
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "`{path}` is not recorded as changed in agent task `{}`",
                    agent_task_label(&view.task)
                ))
            })?;
        let previous = self.latest_agent_file_review_marker(&lane, &change.path)?;
        let note = note.filter(|value| !value.trim().is_empty());
        let payload = serde_json::json!({
            "checkpoint": branch.head_change.0,
            "path": change.path,
            "note": note
        });
        let event_id = self.insert_lane_event_with_context(
            &branch.lane_id,
            view.task.session_id.as_deref(),
            None,
            AGENT_FILE_REVIEWED_EVENT,
            Some(&branch.head_change),
            None,
            &payload,
        )?;
        let marker = self
            .agent_file_review_marker_from_event(self.lane_event(&event_id)?)?
            .ok_or_else(|| {
                Error::InvalidInput("failed to read stored agent file review marker".to_string())
            })?;
        let summary =
            agent_mark_file_reviewed_summary(&view.task, &marker.path, previous.as_ref(), &marker);
        let suggestions = agent_mark_file_reviewed_suggestions(&lane, &marker.path);
        Ok(AgentMarkFileReviewedReport {
            task: view.task,
            lane,
            path: marker.path.clone(),
            marker,
            previous,
            summary,
            suggestions,
        })
    }

    pub fn agent_archive(
        &mut self,
        selector: &str,
        archived: bool,
        note: Option<String>,
    ) -> Result<AgentArchiveReport> {
        let _lock = self.acquire_write_lock()?;
        let lane = if selector == "latest" {
            self.latest_agent_lane()?.ok_or_else(|| {
                Error::InvalidInput("no unarchived agent tasks have been recorded".to_string())
            })?
        } else {
            self.resolve_agent_selector(selector)?.ok_or_else(|| {
                Error::InvalidInput(format!("agent task `{selector}` was not found"))
            })?
        };
        let previous_archived = self.lane_agent_archived(&lane)?;
        let branch = self.lane_branch(&lane)?;
        let note = note.filter(|value| !value.trim().is_empty());
        let event_type = if archived {
            AGENT_TASK_ARCHIVED_EVENT
        } else {
            AGENT_TASK_UNARCHIVED_EVENT
        };
        let payload = serde_json::json!({
            "archived": archived,
            "previous_archived": previous_archived,
            "note": note
        });
        let event_id = self.insert_lane_event_with_context(
            &branch.lane_id,
            None,
            None,
            event_type,
            Some(&branch.head_change),
            None,
            &payload,
        )?;
        let task = self.agent_task_for_lane_details(self.lane_details(&lane)?, 10)?;
        let summary = agent_archive_summary(&task, archived, previous_archived);
        let suggestions = agent_archive_suggestions(&task, archived);
        Ok(AgentArchiveReport {
            task,
            archived,
            previous_archived,
            event_id,
            note,
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
        let view = self.agent_task_view(&lane)?;
        if view.task.status == AgentTaskStatus::Applied {
            return Ok(AgentApplyReport {
                task: view.task.clone(),
                status: "already_applied".to_string(),
                dry_run,
                git_apply_plan: AgentGitApplyPlan {
                    crab_branch,
                    git_branch: None,
                    base_change: target_ref.change_id,
                    result_change: None,
                    range: None,
                    would_record: false,
                    would_create_git_commit: false,
                    would_fast_forward: false,
                },
                recorded: None,
                merge: None,
                git_export: None,
                fast_forwarded: false,
                warnings: vec![
                    "agent task has already been applied; use `crabdb agent continue` for follow-up work to avoid reusing old lane history"
                        .to_string(),
                ],
                suggestions: agent_already_applied_suggestions(&view.task),
            });
        }
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
                let mut suggestions = vec![StatusSuggestion {
                        command: format!("crabdb agent land {lane}"),
                        reason: format!(
                            "create a Git commit using default message `{}` and fast-forward the current branch",
                            default_agent_apply_message_for_task(&view.task)
                        ),
                    }];
                agent_push_suggestion(
                    &mut suggestions,
                    format!("crabdb agent finish {lane}"),
                    "apply the task and hide it from the default inbox after success",
                );
                ("ready".to_string(), suggestions)
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

    pub fn agent_finish(
        &mut self,
        selector: &str,
        dry_run: bool,
        message: Option<String>,
        note: Option<String>,
    ) -> Result<AgentFinishReport> {
        let apply = self.agent_apply(selector, dry_run, message)?;
        let apply_can_finish = matches!(
            apply.status.as_str(),
            "ready" | "would_record" | "applied" | "already_applied"
        );
        let apply_succeeded = matches!(apply.status.as_str(), "applied" | "already_applied");
        let would_archive = dry_run && apply_can_finish && !apply.task.archived;
        let archive = if !dry_run && apply_succeeded && !apply.task.archived {
            let note = note.or_else(|| Some("finished after apply".to_string()));
            Some(self.agent_archive(&apply.task.lane, true, note)?)
        } else {
            None
        };
        let task = archive
            .as_ref()
            .map(|report| report.task.clone())
            .unwrap_or_else(|| apply.task.clone());
        let status = if dry_run {
            if apply_can_finish {
                "ready".to_string()
            } else {
                apply.status.clone()
            }
        } else if apply_succeeded && (archive.is_some() || task.archived) {
            "finished".to_string()
        } else {
            apply.status.clone()
        };
        let suggestions = agent_finish_suggestions(&task, &apply, &status, dry_run);
        Ok(AgentFinishReport {
            task,
            status,
            dry_run,
            apply,
            archive,
            would_archive,
            suggestions,
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
        let archived = self.lane_agent_archived(&lane_name)?;
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
            archived,
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
        for acp in self.list_lane_acp_sessions(None)?.sessions {
            let lane = self.resolve_lane_handle(&acp.lane_id)?;
            if !self.lane_agent_archived(&lane)? {
                return Ok(Some(lane));
            }
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
                if self.lane_agent_archived(&lane.record.name)? {
                    continue;
                }
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

    fn agent_available_commands(&self, view: &AgentTaskViewReport) -> Result<Vec<String>> {
        let mut events = self.list_lane_events(
            Some(&view.task.lane),
            view.task.session_id.as_deref(),
            None,
            Some("acp_available_commands_update"),
            1000,
        )?;
        events.reverse();
        let mut commands = Vec::new();
        let mut seen = BTreeSet::new();
        for event in events {
            let Some(payload) = event.payload.as_ref() else {
                continue;
            };
            let Some(names) = payload
                .get("command_names")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            for name in names {
                let Some(name) = name.as_str() else {
                    continue;
                };
                if seen.insert(name.to_string()) {
                    commands.push(name.to_string());
                }
            }
        }
        Ok(commands)
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
        if task.archived {
            return Ok(StatusSuggestion {
                command: format!("crabdb agent unarchive {}", task.lane),
                reason: "restore this archived task to the default inbox".to_string(),
            });
        }
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
        let (attention, detail) = if task.archived {
            (
                "archived".to_string(),
                "Hidden from the default agent inbox; use `agent unarchive` to restore it."
                    .to_string(),
            )
        } else {
            match &task.status {
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
                        review_first =
                            Some(AgentInboxReviewTarget {
                                path: priority.change.path.clone(),
                                reason: priority.reasons.first().cloned().unwrap_or_else(|| {
                                    "highest-ranked review priority".to_string()
                                }),
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
                        agent_push_suggestion(
                            &mut suggestions,
                            suggestion.command,
                            &suggestion.reason,
                        );
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

    fn latest_agent_file_review_marker(
        &self,
        lane: &str,
        path: &str,
    ) -> Result<Option<AgentFileReviewMarker>> {
        let markers = self.latest_agent_file_review_markers(lane)?;
        Ok(markers
            .into_iter()
            .find(|(_, marker)| marker.path == path)
            .map(|(_, marker)| marker))
    }

    fn latest_agent_file_review_markers(
        &self,
        lane: &str,
    ) -> Result<BTreeMap<String, AgentFileReviewMarker>> {
        let branch = self.lane_branch(lane)?;
        let mut stmt = self.conn.prepare(
            "SELECT event_id, lane_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM lane_events \
             WHERE lane_id = ?1 AND event_type = ?2 \
             ORDER BY created_at DESC, rowid DESC",
        )?;
        let events = stmt
            .query_map(
                params![branch.lane_id, AGENT_FILE_REVIEWED_EVENT],
                lane_event_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut markers = BTreeMap::new();
        for event in events {
            if let Some(marker) = self.agent_file_review_marker_from_event(event)? {
                markers.entry(marker.path.clone()).or_insert(marker);
            }
        }
        Ok(markers)
    }

    fn valid_agent_file_review_markers(
        &self,
        lane: &str,
        head_change: &Option<ChangeId>,
    ) -> Result<BTreeMap<String, AgentFileReviewMarker>> {
        let Some(head_change) = head_change else {
            return Ok(BTreeMap::new());
        };
        let markers = self.latest_agent_file_review_markers(lane)?;
        let mut changed_since_by_checkpoint: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut valid = BTreeMap::new();
        for (path, marker) in markers {
            if marker.checkpoint == *head_change {
                valid.insert(path, marker);
                continue;
            }
            let changed_since = if let Some(paths) =
                changed_since_by_checkpoint.get(marker.checkpoint.0.as_str())
            {
                paths.clone()
            } else {
                let diff =
                    self.diff_refs_with_options(&marker.checkpoint.0, &head_change.0, false, true)?;
                let paths = diff
                    .files
                    .into_iter()
                    .map(|file| file.path)
                    .collect::<BTreeSet<_>>();
                changed_since_by_checkpoint.insert(marker.checkpoint.0.clone(), paths.clone());
                paths
            };
            if !changed_since.iter().any(|changed| changed == &marker.path) {
                valid.insert(path, marker);
            }
        }
        Ok(valid)
    }

    fn lane_agent_archived(&self, lane: &str) -> Result<bool> {
        let branch = self.lane_branch(lane)?;
        let mut stmt = self.conn.prepare(
            "SELECT event_id, lane_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM lane_events \
             WHERE lane_id = ?1 AND event_type IN (?2, ?3) \
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )?;
        let event = stmt
            .query_row(
                params![
                    branch.lane_id,
                    AGENT_TASK_ARCHIVED_EVENT,
                    AGENT_TASK_UNARCHIVED_EVENT
                ],
                lane_event_row,
            )
            .optional()?;
        Ok(matches!(
            event.as_ref().map(|event| event.event_type.as_str()),
            Some(AGENT_TASK_ARCHIVED_EVENT)
        ))
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

    fn agent_file_review_marker_from_event(
        &self,
        event: LaneEventRecord,
    ) -> Result<Option<AgentFileReviewMarker>> {
        if event.event_type != AGENT_FILE_REVIEWED_EVENT {
            return Ok(None);
        }
        let payload = event.payload.as_ref();
        let path = payload
            .and_then(|payload| payload.get("path"))
            .and_then(serde_json::Value::as_str)
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "agent file reviewed marker `{}` is missing path",
                    event.event_id
                ))
            })?
            .to_string();
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
                    "agent file reviewed marker `{}` is missing checkpoint",
                    event.event_id
                ))
            })?;
        let note = payload
            .and_then(|payload| payload.get("note"))
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        Ok(Some(AgentFileReviewMarker {
            event_id: event.event_id,
            path,
            checkpoint,
            reviewed_at: event.created_at,
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
    let mentions_apply_flow = lowered.contains("pull request")
        || lowered.contains("apply")
        || lowered.contains("merge")
        || lowered.contains("ready")
        || agent_ask_has_any(&lowered_tokens, &["land", "ship", "pr"]);
    if lowered.contains("review data")
        || lowered.contains("review json")
        || lowered.contains("review packet json")
        || lowered.contains("editor panel")
        || lowered.contains("side panel")
        || lowered.contains("panel data")
        || lowered.contains("ui data")
        || lowered.contains("one json")
        || lowered.contains("single json")
        || lowered.contains("single packet")
    {
        return Ok(AgentAskRoute::ReviewData);
    }
    let asks_actions = lowered.contains("action palette")
        || lowered.contains("actions palette")
        || lowered.contains("command palette")
        || lowered.contains("show actions")
        || lowered.contains("show action")
        || lowered.contains("list actions")
        || lowered.contains("available actions")
        || lowered.contains("what actions")
        || lowered.contains("which actions")
        || lowered.contains("buttons")
        || lowered.contains("show buttons")
        || lowered.contains("what buttons")
        || lowered.contains("what can i do")
        || lowered.contains("what can we do")
        || lowered.contains("what are my options")
        || lowered.contains("what options")
        || lowered.contains("available commands")
        || lowered.contains("what commands can i run")
        || lowered.contains("which commands can i run");
    if path.is_none() && asks_actions {
        return Ok(AgentAskRoute::Actions);
    }
    let asks_blocker = lowered.contains("what blocks")
        || lowered.contains("what is blocking")
        || lowered.contains("what's blocking")
        || lowered.contains("what is blocked")
        || lowered.contains("why blocked")
        || lowered.contains("why is this blocked")
        || lowered.contains("why is it blocked")
        || lowered.contains("blocking this")
        || lowered.contains("blocking the")
        || agent_ask_has_any(&lowered_tokens, &["blockers", "blocking"]);
    let asks_problem = lowered.contains("what went wrong")
        || lowered.contains("what's wrong")
        || lowered.contains("what is wrong")
        || lowered.contains("anything wrong")
        || lowered.contains("any problems")
        || lowered.contains("any issues")
        || lowered.contains("did it fail")
        || lowered.contains("did this fail")
        || lowered.contains("why did it fail")
        || lowered.contains("why did this fail")
        || lowered.contains("why failed")
        || lowered.contains("why is it failing")
        || lowered.contains("why is this failing")
        || agent_ask_has_any(
            &lowered_tokens,
            &["failed", "failing", "failure", "problem", "problems"],
        );
    let asks_file_risk = (lowered.contains("risk")
        || lowered.contains("risky")
        || lowered.contains("red flag")
        || lowered.contains("worry")
        || lowered.contains("danger"))
        && (lowered.contains("file")
            || lowered.contains("files")
            || lowered.contains("path")
            || lowered.contains("paths"));
    let asks_file_to_open = lowered.contains("what file should i open")
        || lowered.contains("which file should i open")
        || lowered.contains("what file do i open")
        || lowered.contains("which file do i open")
        || lowered.contains("file should i open")
        || lowered.contains("open first file")
        || lowered.contains("open the first file")
        || lowered.contains("open next file")
        || lowered.contains("open the next file")
        || lowered.contains("open in editor")
        || lowered.contains("open in my editor");
    let asks_impact = lowered.contains("impact")
        || lowered.contains("blast radius")
        || lowered.contains("surface area")
        || lowered.contains("scope of change")
        || lowered.contains("change scope")
        || lowered.contains("what areas")
        || lowered.contains("which areas")
        || lowered.contains("areas did")
        || lowered.contains("areas changed")
        || lowered.contains("what parts")
        || lowered.contains("which parts")
        || lowered.contains("what surfaces")
        || lowered.contains("which surfaces")
        || lowered.contains("what should i test because")
        || lowered.contains("what should we test because");
    let asks_review_map = lowered.contains("review map")
        || lowered.contains("review-map")
        || lowered.contains("file checklist")
        || lowered.contains("files checklist")
        || lowered.contains("review files")
        || lowered.contains("review all files")
        || lowered.contains("review by file")
        || lowered.contains("review by area")
        || lowered.contains("map of changes")
        || lowered.contains("map the changes")
        || lowered.contains("change map")
        || lowered.contains("changes map")
        || lowered.contains("review every file");
    let asks_confidence = lowered.contains("confidence")
        || lowered.contains("go/no-go")
        || lowered.contains("go no go")
        || lowered.contains("go-no-go")
        || lowered.contains("final check")
        || lowered.contains("ship check")
        || lowered.contains("apply check")
        || lowered.contains("green light")
        || lowered.contains("am i good")
        || lowered.contains("are we good")
        || lowered.contains("good to go")
        || lowered.contains("should i ship")
        || lowered.contains("should we ship")
        || lowered.contains("should i land")
        || lowered.contains("should we land")
        || lowered.contains("should i apply")
        || lowered.contains("should we apply");
    let asks_test_plan = lowered.contains("test plan")
        || lowered.contains("validation plan")
        || lowered.contains("what tests")
        || lowered.contains("which tests")
        || lowered.contains("what should i test")
        || lowered.contains("what should we test")
        || lowered.contains("how do i test")
        || lowered.contains("how should i test")
        || lowered.contains("how should we test")
        || lowered.contains("how to test")
        || lowered.contains("test this")
        || lowered.contains("test it")
        || lowered.contains("run tests")
        || lowered.contains("run the tests")
        || lowered.contains("what validation should i run")
        || lowered.contains("which validation should i run");

    if path.is_none() && asks_confidence {
        return Ok(AgentAskRoute::Confidence);
    }
    if path.is_none() && asks_impact {
        return Ok(AgentAskRoute::Impact);
    }
    if path.is_none() && asks_review_map {
        return Ok(AgentAskRoute::ReviewMap);
    }
    if path.is_none() && asks_test_plan {
        return Ok(AgentAskRoute::TestPlan);
    }
    if path.is_none()
        && mentions_apply_flow
        && (agent_ask_has_any(&lowered_tokens, &["why", "reason"])
            || lowered.contains("why can't")
            || lowered.contains("why cant")
            || lowered.contains("why cannot")
            || asks_blocker)
    {
        return Ok(AgentAskRoute::Ready);
    }
    if path.is_none() && asks_blocker {
        return Ok(AgentAskRoute::Diagnose);
    }
    if path.is_none() && asks_problem {
        return Ok(AgentAskRoute::Diagnose);
    }
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
    if asks_file_risk {
        return Ok(AgentAskRoute::ChangesByFile);
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
        || lowered.contains("what did it change")
        || lowered.contains("what did the agent change")
        || lowered.contains("what did it touch")
        || lowered.contains("what did the agent touch")
        || lowered.contains("what files did it change")
        || lowered.contains("what files did the agent change")
        || lowered.contains("which files did it change")
        || lowered.contains("which files did the agent change")
        || lowered.contains("what files did it touch")
        || lowered.contains("what files did the agent touch")
        || lowered.contains("which files did it touch")
        || lowered.contains("which files did the agent touch")
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
    if lowered.contains("apply order")
        || lowered.contains("apply first")
        || lowered.contains("which agent first")
        || lowered.contains("which task first")
        || lowered.contains("what should i apply first")
        || lowered.contains("what should i land first")
        || lowered.contains("what should i finish first")
        || lowered.contains("show stack")
        || lowered.contains("agent stack")
        || lowered.contains("task stack")
        || matches!(lowered.trim(), "stack" | "order" | "apply order")
    {
        return Ok(AgentAskRoute::Stack);
    }
    if lowered.contains("agent board")
        || lowered.contains("task board")
        || lowered.contains("multi agent")
        || lowered.contains("multi-agent")
        || lowered.contains("show board")
        || matches!(lowered.trim(), "board" | "tasks")
    {
        return Ok(AgentAskRoute::Board);
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
    if lowered.contains("walk me through")
        || lowered.contains("walk through")
        || lowered.contains("walkthrough")
        || lowered.contains("step by step")
        || lowered.contains("step-by-step")
        || lowered.contains("review flow")
        || lowered.contains("review loop")
        || lowered.contains("review checklist")
        || lowered.contains("finish checklist")
        || lowered.contains("ship checklist")
        || lowered.contains("guide me through review")
        || lowered.contains("guide me through the review")
        || lowered.contains("how do i review")
        || lowered.contains("how should i review")
        || lowered.contains("review steps")
        || lowered.contains("review workflow")
    {
        return Ok(AgentAskRoute::ReviewFlow);
    }
    if lowered.contains("review first")
        || lowered.contains("inspect first")
        || asks_file_to_open
        || lowered.contains("what file should i review first")
        || lowered.contains("which file should i review first")
        || lowered.contains("first file to review")
        || lowered.contains("first file should i review")
        || lowered.contains("look at first")
        || lowered.contains("look first")
        || lowered.contains("where should i look first")
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
    if lowered.contains("commit message")
        || lowered.contains("git message")
        || lowered.contains("message for commit")
        || lowered.contains("message should i use")
        || lowered.contains("what message should i use")
        || lowered.contains("commit title")
    {
        return Ok(AgentAskRoute::Ready);
    }
    if lowered.contains("pr title")
        || lowered.contains("pr body")
        || lowered.contains("pr description")
        || lowered.contains("pull request title")
        || lowered.contains("pull request body")
        || lowered.contains("pull request description")
        || lowered.contains("draft pr")
        || lowered.contains("draft a pr")
        || lowered.contains("draft the pr")
        || lowered.contains("draft pull request")
        || lowered.contains("draft a pull request")
        || lowered.contains("draft the pull request")
        || lowered.contains("put in the pr")
        || lowered.contains("put in pr")
        || lowered.contains("put in the pull request")
        || lowered.contains("put in pull request")
        || lowered.contains("write the pr")
        || lowered.contains("write a pr")
        || lowered.contains("write the pull request")
        || lowered.contains("write a pull request")
    {
        return Ok(AgentAskRoute::Pr);
    }
    if lowered.contains("handoff")
        || lowered.contains("hand off")
        || lowered.contains("share with another agent")
        || lowered.contains("share with an agent")
        || lowered.contains("give to another agent")
        || lowered.contains("give this to another agent")
        || lowered.contains("send to another agent")
        || lowered.contains("handoff packet")
    {
        return Ok(AgentAskRoute::Handoff);
    }
    if lowered.contains("receipt")
        || lowered.contains("copyable")
        || lowered.contains("share summary")
        || lowered.contains("summary to share")
        || lowered.contains("what should i share")
        || lowered.contains("note to share")
        || lowered.contains("review note")
        || lowered.contains("after action")
        || lowered.contains("after-action")
        || lowered.contains("post run")
        || lowered.contains("post-run")
    {
        return Ok(AgentAskRoute::Receipt);
    }
    if lowered.contains("red flag")
        || lowered.contains("what should i worry")
        || lowered.contains("what should we worry")
        || lowered.contains("worry about")
        || lowered.contains("worried about")
        || lowered.contains("anything risky")
        || lowered.contains("what is risky")
        || lowered.contains("what's risky")
        || lowered.contains("risky")
        || lowered.contains("dangerous")
        || lowered.contains("danger")
        || lowered.contains("unsafe")
        || lowered.contains("blast radius")
        || lowered.contains("high risk")
        || lowered.contains("risk review")
    {
        return Ok(AgentAskRoute::Risk);
    }
    if lowered.contains("help me")
        || lowered.contains("show guide")
        || lowered.contains("agent guide")
        || lowered.contains("getting started")
        || lowered.contains("how do i use crabdb")
        || lowered.contains("how should i use crabdb")
        || lowered.contains("how to use crabdb")
        || lowered.contains("how do i use this")
        || lowered.contains("how should i use this")
        || lowered.contains("what commands should i use")
        || matches!(lowered.trim(), "help" | "guide")
    {
        return Ok(AgentAskRoute::Guide);
    }
    if lowered.contains("what should")
        || lowered.contains("what now")
        || lowered.contains("next")
        || agent_ask_has_any(&lowered_tokens, &["todo"])
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
        if lowered.contains("by file")
            || lowered.contains("by changed file")
            || lowered.contains("per file")
            || lowered.contains("file by file")
            || lowered.contains("file-by-file")
        {
            return Ok(AgentAskRoute::ChangesByFile);
        }
        return Ok(AgentAskRoute::Changes);
    }
    if wants_patch {
        return Ok(AgentAskRoute::TaskDiff {
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
    if lowered.contains("tool call")
        || lowered.contains("tool use")
        || lowered.contains("tools used")
        || lowered.contains("used tools")
        || lowered.contains("available command")
        || lowered.contains("available commands")
        || agent_ask_has_any(&lowered_tokens, &["tools", "tool"])
    {
        return Ok(AgentAskRoute::Tools);
    }
    if agent_ask_has_any(&lowered_tokens, &["commands", "command"]) {
        return Ok(AgentAskRoute::Tools);
    }
    if lowered.contains("test status")
        || lowered.contains("validation")
        || lowered.contains("tests passing")
        || lowered.contains("tests pass")
        || lowered.contains("test pass")
        || lowered.contains("did tests pass")
        || lowered.contains("did the tests pass")
        || lowered.contains("did it pass tests")
        || lowered.contains("is it tested")
        || lowered.contains("is this tested")
        || lowered.contains("has it been tested")
        || lowered.contains("was it tested")
        || lowered.contains("test results")
        || lowered.contains("validation status")
        || lowered.contains("validation guidance")
        || lowered.contains("missing validation")
        || lowered.contains("validation missing")
        || lowered.contains("what validation")
        || lowered.contains("which validation")
        || lowered.contains("need validation")
        || lowered.contains("needs validation")
        || lowered.contains("do i need tests")
        || lowered.contains("need tests")
    {
        return Ok(AgentAskRoute::Validate);
    }
    if lowered.contains("dashboard")
        || lowered.contains("overview")
        || lowered.contains("cockpit")
        || lowered.contains("status board")
        || lowered.contains("one screen")
    {
        return Ok(AgentAskRoute::Dashboard);
    }
    if lowered.contains("summary") {
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

fn agent_empty_action_palette_value() -> serde_json::Value {
    serde_json::json!({
        "status": "empty",
        "task": null,
        "summary": "No agent task is recorded yet. Set up an editor, verify the provider, or start a terminal task.",
        "next": {
            "command": "crabdb agent setup",
            "reason": "print a stable editor config that creates fresh CrabDB tasks automatically"
        },
        "actions": agent_empty_action_palette_actions()
    })
}

fn agent_empty_action_palette_actions() -> Vec<AgentReviewAction> {
    vec![
        agent_static_action(
            "setup_vscode",
            "Set up VS Code",
            "setup",
            "crabdb agent setup",
            "print a copyable ACP editor config that creates fresh task lanes automatically",
            "read_only",
            false,
        ),
        agent_static_action(
            "doctor_claude_code",
            "Check Claude Code",
            "doctor",
            "crabdb agent doctor --provider claude-code",
            "verify CrabDB workspace readiness and provider availability",
            "read_only",
            false,
        ),
        agent_static_action(
            "start_terminal_task",
            "Start terminal task",
            "start",
            "crabdb agent start --provider claude-code",
            "launch a fresh materialized terminal task when you are not using an editor",
            "open_world",
            true,
        ),
    ]
}

fn agent_static_action(
    id: &str,
    label: &str,
    kind: &str,
    command: &str,
    reason: &str,
    safety: &str,
    requires_confirmation: bool,
) -> AgentReviewAction {
    AgentReviewAction {
        id: id.to_string(),
        label: label.to_string(),
        kind: kind.to_string(),
        command: command.to_string(),
        reason: reason.to_string(),
        enabled: true,
        disabled_reason: None,
        safety: safety.to_string(),
        requires_confirmation,
        path: None,
        open_path: None,
        mcp_tool: None,
        mcp_arguments: None,
    }
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

fn agent_board_item_from_inbox(item: &AgentInboxItem) -> AgentBoardItem {
    AgentBoardItem {
        task: item.task.clone(),
        status_label: agent_task_status_label(&item.task.status).to_string(),
        attention: item.attention.clone(),
        detail: item.detail.clone(),
        changed_paths: item.task.changed_paths.len(),
        turns: item.task.turns,
        tool_events: item.task.tool_events,
        review_first: item.review_first.clone(),
        next: item.next.clone(),
        suggestions: item.suggestions.clone(),
    }
}

fn agent_board_columns(items: &[AgentBoardItem]) -> Vec<AgentBoardColumn> {
    agent_board_column_specs()
        .into_iter()
        .filter_map(|spec| {
            let column_items = items
                .iter()
                .filter(|item| agent_board_item_matches_column(item, spec.key))
                .cloned()
                .collect::<Vec<_>>();
            if column_items.is_empty() {
                return None;
            }
            let next = column_items.first().map(|item| item.next.clone());
            Some(AgentBoardColumn {
                key: spec.key.to_string(),
                label: spec.label.to_string(),
                summary: spec.summary.to_string(),
                attention: spec.attention,
                items: column_items,
                next,
            })
        })
        .collect()
}

struct AgentBoardColumnSpec {
    key: &'static str,
    label: &'static str,
    summary: &'static str,
    attention: bool,
}

fn agent_board_column_specs() -> Vec<AgentBoardColumnSpec> {
    vec![
        AgentBoardColumnSpec {
            key: "needs_record",
            label: "Needs record",
            summary: "Materialized workdirs have unrecorded changes.",
            attention: true,
        },
        AgentBoardColumnSpec {
            key: "conflicted",
            label: "Conflicted",
            summary: "Tasks cannot be applied cleanly yet.",
            attention: true,
        },
        AgentBoardColumnSpec {
            key: "blocked",
            label: "Blocked",
            summary: "Tasks have readiness blockers.",
            attention: true,
        },
        AgentBoardColumnSpec {
            key: "needs_review",
            label: "Needs review",
            summary: "Tasks are ready for a human review pass.",
            attention: true,
        },
        AgentBoardColumnSpec {
            key: "ready",
            label: "Ready to apply",
            summary: "Tasks have been reviewed and can move toward apply.",
            attention: true,
        },
        AgentBoardColumnSpec {
            key: "running",
            label: "Running",
            summary: "Tasks are active or have no checkpoint yet.",
            attention: false,
        },
        AgentBoardColumnSpec {
            key: "applied",
            label: "Applied",
            summary: "Tasks have already landed in Git or CrabDB main.",
            attention: false,
        },
        AgentBoardColumnSpec {
            key: "archived",
            label: "Archived",
            summary: "Tasks hidden from default views.",
            attention: false,
        },
        AgentBoardColumnSpec {
            key: "empty",
            label: "Empty",
            summary: "Setup placeholders with no agent work yet.",
            attention: false,
        },
    ]
}

fn agent_board_item_matches_column(item: &AgentBoardItem, key: &str) -> bool {
    if item.task.archived {
        return key == "archived";
    }
    match key {
        "needs_record" => item.task.status == AgentTaskStatus::Dirty,
        "conflicted" => item.task.status == AgentTaskStatus::Conflicted,
        "blocked" => item.task.status == AgentTaskStatus::Blocked,
        "needs_review" => {
            item.task.status == AgentTaskStatus::Ready && item.attention != "up_to_date"
        }
        "ready" => item.task.status == AgentTaskStatus::Ready && item.attention == "up_to_date",
        "running" => item.task.status == AgentTaskStatus::Active,
        "applied" => item.task.status == AgentTaskStatus::Applied,
        "empty" => item.task.status == AgentTaskStatus::Empty,
        _ => false,
    }
}

fn agent_task_status_label(status: &AgentTaskStatus) -> &'static str {
    match status {
        AgentTaskStatus::Empty => "empty",
        AgentTaskStatus::Active => "active",
        AgentTaskStatus::Dirty => "dirty",
        AgentTaskStatus::Ready => "ready",
        AgentTaskStatus::Blocked => "blocked",
        AgentTaskStatus::Conflicted => "conflicted",
        AgentTaskStatus::Applied => "applied",
    }
}

fn agent_inbox_next_for_task(task: &AgentTaskReport) -> StatusSuggestion {
    let lane = &task.lane;
    match &task.status {
        AgentTaskStatus::Empty => StatusSuggestion {
            command: "crabdb agent setup".to_string(),
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
            command: format!("crabdb agent finish {lane}"),
            reason: "hide the applied task from the default inbox when you are done".to_string(),
        },
    }
}

fn agent_stack_shared_paths(tasks: &[AgentTaskReport]) -> Vec<AgentStackSharedPath> {
    let mut owners: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for task in tasks {
        let mut seen = BTreeSet::new();
        for path in &task.changed_paths {
            if seen.insert(path.path.clone()) {
                owners
                    .entry(path.path.clone())
                    .or_default()
                    .push((task.lane.clone(), agent_task_label(task).to_string()));
            }
        }
    }
    owners
        .into_iter()
        .filter_map(|(path, owners)| {
            if owners.len() < 2 {
                return None;
            }
            let (lanes, task_titles): (Vec<_>, Vec<_>) = owners.into_iter().unzip();
            Some(AgentStackSharedPath {
                path,
                lanes,
                task_titles,
            })
        })
        .collect()
}

fn agent_stack_shared_paths_by_lane(
    shared_paths: &[AgentStackSharedPath],
) -> BTreeMap<String, Vec<String>> {
    let mut by_lane: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for shared in shared_paths {
        for lane in &shared.lanes {
            by_lane
                .entry(lane.clone())
                .or_default()
                .push(shared.path.clone());
        }
    }
    by_lane
}

fn agent_stack_item_status(
    task: &AgentTaskReport,
    risk: &AgentRiskReport,
    has_overlap: bool,
) -> String {
    if task.archived {
        return "archived".to_string();
    }
    if matches!(risk.level, AgentRiskLevel::Blocking) {
        return "blocked".to_string();
    }
    match task.status {
        AgentTaskStatus::Ready | AgentTaskStatus::Dirty if has_overlap => {
            "overlap_review".to_string()
        }
        AgentTaskStatus::Ready => "ready".to_string(),
        AgentTaskStatus::Dirty => "needs_record".to_string(),
        AgentTaskStatus::Blocked | AgentTaskStatus::Conflicted => "blocked".to_string(),
        AgentTaskStatus::Applied => "applied".to_string(),
        AgentTaskStatus::Active => "active".to_string(),
        AgentTaskStatus::Empty => "empty".to_string(),
    }
}

fn agent_stack_item_applyable(
    task: &AgentTaskReport,
    risk: &AgentRiskReport,
    shared_paths: &[String],
) -> bool {
    !task.archived
        && shared_paths.is_empty()
        && matches!(task.status, AgentTaskStatus::Ready | AgentTaskStatus::Dirty)
        && !matches!(risk.level, AgentRiskLevel::Blocking)
}

fn agent_stack_item_next(task: &AgentTaskReport, status: &str) -> StatusSuggestion {
    let lane = &task.lane;
    match status {
        "ready" | "needs_record" => StatusSuggestion {
            command: format!("crabdb agent finish {lane} --dry-run"),
            reason: "preview applying and closing this task".to_string(),
        },
        "overlap_review" => StatusSuggestion {
            command: format!("crabdb agent changes {lane} --by-file"),
            reason: "review shared changed files before applying".to_string(),
        },
        "blocked" => StatusSuggestion {
            command: format!("crabdb agent recover {lane}"),
            reason: "inspect blockers before choosing an apply order".to_string(),
        },
        "applied" => StatusSuggestion {
            command: format!("crabdb agent finish {lane}"),
            reason: "hide the already-applied task from the default inbox".to_string(),
        },
        "active" => StatusSuggestion {
            command: format!("crabdb agent view {lane}"),
            reason: "inspect current task activity before applying".to_string(),
        },
        "archived" => StatusSuggestion {
            command: format!("crabdb agent receipt {lane}"),
            reason: "inspect the archived task receipt if needed".to_string(),
        },
        _ => StatusSuggestion {
            command: "crabdb agent inbox".to_string(),
            reason: "return to the agent task inbox".to_string(),
        },
    }
}

fn agent_stack_item_sort_key(item: &AgentStackItem) -> (u8, u8, u8, usize, String) {
    let status_rank = match item.status.as_str() {
        "overlap_review" => 0,
        "blocked" => 1,
        "ready" => 2,
        "needs_record" => 3,
        "applied" => 4,
        "active" => 5,
        "empty" => 6,
        "archived" => 7,
        _ => 8,
    };
    (
        status_rank,
        agent_risk_sort_rank(&item.risk.level),
        item.risk.score,
        item.task.changed_paths.len(),
        item.task.lane.clone(),
    )
}

fn agent_risk_sort_rank(level: &AgentRiskLevel) -> u8 {
    match level {
        AgentRiskLevel::Low => 0,
        AgentRiskLevel::Medium => 1,
        AgentRiskLevel::High => 2,
        AgentRiskLevel::Blocking => 3,
    }
}

fn agent_stack_next(
    items: &[AgentStackItem],
    shared_paths: &[AgentStackSharedPath],
) -> StatusSuggestion {
    if let Some(shared) = shared_paths.first() {
        if shared.lanes.len() >= 2 {
            return StatusSuggestion {
                command: format!(
                    "crabdb agent compare {} {}",
                    shared.lanes[0], shared.lanes[1]
                ),
                reason: format!(
                    "review overlap on `{}` before applying either task",
                    shared.path
                ),
            };
        }
    }
    if let Some(item) = items.iter().find(|item| item.status == "blocked") {
        return item.next.clone();
    }
    if let Some(item) = items.iter().find(|item| item.applyable) {
        return item.next.clone();
    }
    if let Some(item) = items.iter().find(|item| item.status == "applied") {
        return item.next.clone();
    }
    if let Some(item) = items.iter().find(|item| item.status == "active") {
        return item.next.clone();
    }
    StatusSuggestion {
        command: "crabdb agent inbox".to_string(),
        reason: "choose an agent task from the inbox".to_string(),
    }
}

fn agent_stack_summary(
    total: usize,
    ready_count: usize,
    blocked_count: usize,
    overlap_count: usize,
) -> String {
    if total == 0 {
        return "No agent tasks are recorded yet.".to_string();
    }
    if overlap_count > 0 {
        return format!(
            "{total} task(s), {ready_count} apply candidate(s), {blocked_count} blocked/conflicted, and {overlap_count} shared changed path(s). Resolve overlap before applying."
        );
    }
    if ready_count > 0 {
        return format!(
            "{total} task(s), {ready_count} apply candidate(s), and {blocked_count} blocked/conflicted. Apply candidates are ordered by low risk and smaller change size."
        );
    }
    format!("{total} task(s), no apply candidates, {blocked_count} blocked/conflicted.")
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

fn agent_guide_headline(task: Option<&AgentTaskReport>, status: &AgentTaskStatus) -> String {
    match task {
        Some(task) => format!(
            "Use `{}` as one agent task: inspect it, validate it, apply it, or recover it.",
            task.title
        ),
        None => match status {
            AgentTaskStatus::Empty => {
                "Set up one editor or terminal entrypoint; CrabDB will create fresh tasks automatically."
                    .to_string()
            }
            _ => "Use CrabDB agent commands to inspect, validate, apply, or recover agent work."
                .to_string(),
        },
    }
}

fn agent_guide_current_state(task: Option<&AgentTaskReport>, next: &AgentNextReport) -> String {
    match task {
        Some(task) => format!(
            "Task `{}` is {:?} with {} changed file(s), {} turn(s), and {} tool event(s). Next: {}.",
            task.title,
            task.status,
            task.changed_paths.len(),
            task.turns,
            task.tool_events,
            next.primary.reason
        ),
        None => format!("No agent task is recorded yet. Next: {}.", next.primary.reason),
    }
}

fn agent_guide_steps(
    task: Option<&AgentTaskReport>,
    primary: &StatusSuggestion,
) -> Vec<AgentGuideStep> {
    let Some(task) = task else {
        return vec![
            agent_guide_step(
                "Connect an editor",
                "crabdb agent setup",
                "print a stable ACP editor config that creates fresh CrabDB tasks automatically",
                "once per editor setup",
            ),
            agent_guide_step(
                "Check the provider",
                "crabdb agent doctor --provider claude-code",
                "verify CrabDB and the provider before the first real session",
                "before or after setup when something does not connect",
            ),
            agent_guide_step(
                "Start in terminal",
                "crabdb agent start --provider claude-code",
                "launch a fresh materialized terminal task when you are not using an ACP editor",
                "when you want to work from the shell",
            ),
        ];
    };

    let lane = &task.lane;
    let provider = task.provider.as_deref().unwrap_or("claude-code");
    let mut steps = vec![agent_guide_step(
        "Do next",
        primary.command.clone(),
        primary.reason.clone(),
        "right now",
    )];
    agent_push_guide_step(
        &mut steps,
        "Show actions",
        format!("crabdb agent action {lane}"),
        "list the safe review, validation, apply, and recovery actions CrabDB can run for this task",
        "whenever you want a small command palette instead of remembering commands",
    );

    match task.status {
        AgentTaskStatus::Active => {
            steps.push(agent_guide_step(
                "Watch the task",
                format!("crabdb agent dashboard {lane}"),
                "see the current task, focus file, validation, and next action in one screen",
                "while the editor or terminal agent is still running",
            ));
            steps.push(agent_guide_step(
                "Read the transcript",
                format!("crabdb agent view {lane}"),
                "inspect prompts, assistant messages, tools, changed files, and checkpoint data",
                "when you want to understand what the agent is doing",
            ));
        }
        AgentTaskStatus::Dirty => {
            agent_push_guide_step(
                &mut steps,
                "Preview apply",
                format!("crabdb agent land {lane} --dry-run"),
                "record dirty workdir changes and preview the Git apply plan without mutating Git",
                "after the agent changed files but before applying",
            );
            steps.push(agent_guide_step(
                "Review changes",
                format!("crabdb agent changes {lane} --by-file"),
                "review changed files with the prompt, tools, and diff commands behind each file",
                "before approving the task",
            ));
        }
        AgentTaskStatus::Ready => {
            steps.push(agent_guide_step(
                "Review changes",
                format!("crabdb agent changes {lane}"),
                "see high-level change cards connected to turns, checkpoints, and files",
                "before applying the task",
            ));
            steps.push(agent_guide_step(
                "Validate",
                format!("crabdb agent validate {lane}"),
                "see missing test/eval gates and copy the recommended validation command",
                "before applying when tests matter",
            ));
            steps.push(agent_guide_step(
                "Apply safely",
                format!("crabdb agent land {lane} --dry-run"),
                "preview the Git commit and fast-forward plan before the real apply",
                "when review looks good",
            ));
        }
        AgentTaskStatus::Blocked | AgentTaskStatus::Conflicted => {
            steps.push(agent_guide_step(
                "Diagnose",
                format!("crabdb agent recover {lane}"),
                "identify the blocker and list safe recovery options before destructive commands",
                "when the task is blocked, conflicted, or sideways",
            ));
            steps.push(agent_guide_step(
                "Pick a recovery point",
                format!("crabdb agent rewind-points {lane}"),
                "list friendly checkpoint targets such as before-last-turn or before-prompt text",
                "before running rewind or undo",
            ));
        }
        AgentTaskStatus::Applied => {
            steps.push(agent_guide_step(
                "Inspect receipt",
                format!("crabdb agent receipt {lane}"),
                "copy the applied task summary, validation, changed files, turns, tools, and checkpoint",
                "after a task has landed",
            ));
            steps.push(agent_guide_step(
                "Close task",
                format!("crabdb agent close {lane}"),
                "hide the applied task from default inbox/list/latest views without deleting provenance",
                "after you no longer need it in the daily inbox",
            ));
            steps.push(agent_guide_step(
                "Start follow-up",
                format!("crabdb agent continue {lane} --provider {provider}"),
                "create a fresh task from this applied checkpoint instead of reusing already-applied lane history",
                "when you want more edits",
            ));
        }
        AgentTaskStatus::Empty => {}
    }

    agent_push_guide_step(
        &mut steps,
        "Ask naturally",
        format!(
            "crabdb agent ask --selector {lane} {}",
            agent_shell_arg("what should I do next")
        ),
        "route a plain-language question to the right CrabDB view",
        "whenever you forget the exact command",
    );
    agent_push_guide_step(
        &mut steps,
        "Open review focus",
        format!("crabdb agent focus {lane}"),
        "show the highest-priority file, why it changed, and the focused diff command",
        "when you want one file to inspect first",
    );
    agent_push_guide_step(
        &mut steps,
        "See new work",
        format!("crabdb agent what-changed {lane}"),
        "show only changes since your last reviewed marker",
        "after follow-up prompts",
    );
    steps
}

fn agent_guide_step(
    label: impl Into<String>,
    command: impl Into<String>,
    reason: impl Into<String>,
    when: impl Into<String>,
) -> AgentGuideStep {
    AgentGuideStep {
        label: label.into(),
        command: command.into(),
        reason: reason.into(),
        when: when.into(),
    }
}

fn agent_push_guide_step(
    steps: &mut Vec<AgentGuideStep>,
    label: impl Into<String>,
    command: impl Into<String>,
    reason: impl Into<String>,
    when: impl Into<String>,
) {
    let command = command.into();
    if steps.iter().any(|step| step.command == command) {
        return;
    }
    steps.push(agent_guide_step(label, command, reason, when));
}

fn agent_guide_concepts() -> Vec<AgentGuideConcept> {
    vec![
        AgentGuideConcept {
            name: "Agent task".to_string(),
            meaning:
                "one agent job with its own isolated CrabDB lane, transcript, tools, and checkpoints"
                    .to_string(),
        },
        AgentGuideConcept {
            name: "Changes".to_string(),
            meaning:
                "the review map that connects prompts, files, tools, checkpoints, and apply readiness"
                    .to_string(),
        },
        AgentGuideConcept {
            name: "Apply".to_string(),
            meaning:
                "the safe path back to Git: preview, create a Git commit, then fast-forward only when clean"
                    .to_string(),
        },
        AgentGuideConcept {
            name: "Recover".to_string(),
            meaning:
                "diagnose, choose a friendly checkpoint, then undo or rewind when an agent goes sideways"
                    .to_string(),
        },
    ]
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

#[derive(Default)]
struct AgentToolAccumulator {
    name: String,
    kind: Option<String>,
    event_count: usize,
    changed_paths: BTreeMap<String, FileDiffSummary>,
    event_types: BTreeSet<String>,
    statuses: BTreeMap<String, usize>,
    first_seen_at: Option<i64>,
    last_seen_at: Option<i64>,
    turns: BTreeMap<usize, AgentToolTurnRef>,
}

fn agent_tool_entries(
    lane: &str,
    view: &AgentTaskViewReport,
    group_by_turn: &BTreeMap<String, AgentChangeGroup>,
) -> Vec<AgentToolEntry> {
    let Some(transcript) = &view.transcript else {
        return Vec::new();
    };
    let tool_names_by_id = agent_tool_names_by_id(transcript);
    let mut tools: BTreeMap<String, AgentToolAccumulator> = BTreeMap::new();
    for (turn_idx, turn) in transcript.turns.iter().enumerate() {
        let index = turn_idx + 1;
        let group = group_by_turn.get(&turn.turn.turn_id);
        let changed_paths = group
            .map(|group| group.changed_paths.clone())
            .unwrap_or_default();
        for event in &turn.events {
            if !agent_is_tool_event(event) {
                continue;
            }
            let name = agent_tool_event_name(event, &tool_names_by_id);
            let entry = tools
                .entry(name.clone())
                .or_insert_with(|| AgentToolAccumulator {
                    name: name.clone(),
                    ..AgentToolAccumulator::default()
                });
            entry.event_count += 1;
            entry.event_types.insert(event.event_type.clone());
            if entry.kind.is_none() {
                entry.kind = agent_tool_event_kind(event);
            }
            if let Some(status) = agent_tool_event_status(event) {
                *entry.statuses.entry(status).or_insert(0) += 1;
            }
            entry.first_seen_at = Some(
                entry
                    .first_seen_at
                    .map(|seen| seen.min(event.created_at))
                    .unwrap_or(event.created_at),
            );
            entry.last_seen_at = Some(
                entry
                    .last_seen_at
                    .map(|seen| seen.max(event.created_at))
                    .unwrap_or(event.created_at),
            );
            for change in &changed_paths {
                entry
                    .changed_paths
                    .entry(change.path.clone())
                    .or_insert_with(|| change.clone());
            }
            entry
                .turns
                .entry(index)
                .or_insert_with(|| AgentToolTurnRef {
                    index,
                    turn_id: turn.turn.turn_id.clone(),
                    status: turn.turn.status.clone(),
                    prompt_preview: turn_prompt_preview(turn),
                    checkpoint: group
                        .and_then(|group| group.checkpoint.clone())
                        .or_else(|| turn.checkpoint.clone())
                        .or_else(|| turn.turn.after_change.clone()),
                    changed_paths: changed_paths.clone(),
                    turn_command: format!("crabdb agent turn {lane} {index}"),
                    diff_command: group
                        .and_then(|group| group.after_change.as_ref())
                        .map(|_| agent_turn_diff_command(lane, Some(index), None, true)),
                });
        }
    }
    let mut entries = tools
        .into_values()
        .map(|tool| AgentToolEntry {
            rank: 0,
            name: tool.name,
            kind: tool.kind,
            event_count: tool.event_count,
            turn_count: tool.turns.len(),
            changed_paths: tool.changed_paths.into_values().collect(),
            event_types: tool.event_types.into_iter().collect(),
            statuses: tool.statuses,
            first_seen_at: tool.first_seen_at.unwrap_or(0),
            last_seen_at: tool.last_seen_at.unwrap_or(0),
            turns: tool.turns.into_values().collect(),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .event_count
            .cmp(&left.event_count)
            .then_with(|| right.turn_count.cmp(&left.turn_count))
            .then_with(|| left.name.cmp(&right.name))
    });
    for (idx, entry) in entries.iter_mut().enumerate() {
        entry.rank = idx + 1;
    }
    entries
}

fn agent_tool_names_by_id(transcript: &TranscriptReport) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for turn in &transcript.turns {
        for event in &turn.events {
            let Some(id) = agent_tool_event_identity(event) else {
                continue;
            };
            let Some(name) = agent_tool_event_title(event) else {
                continue;
            };
            names.entry(id).or_insert(name);
        }
    }
    names
}

fn agent_is_tool_event(event: &LaneEventRecord) -> bool {
    match event.event_type.as_str() {
        "tool_call" | "tool_call_update" => true,
        "span_started" => {
            event
                .payload
                .as_ref()
                .and_then(|payload| payload.get("span_type"))
                .and_then(serde_json::Value::as_str)
                == Some("tool")
        }
        "span_ended" => agent_tool_event_identity(event).is_some(),
        _ => false,
    }
}

fn agent_tool_event_name(
    event: &LaneEventRecord,
    tool_names_by_id: &BTreeMap<String, String>,
) -> String {
    if let Some(title) = agent_tool_event_title(event) {
        return title;
    }
    if let Some(id) = agent_tool_event_identity(event) {
        if let Some(title) = tool_names_by_id.get(&id) {
            return title.clone();
        }
        return id;
    }
    event.event_type.clone()
}

fn agent_tool_event_identity(event: &LaneEventRecord) -> Option<String> {
    let payload = event.payload.as_ref()?;
    for key in ["toolCallId", "tool_call_id", "tool_id", "id", "span_id"] {
        if let Some(value) = payload.get(key).and_then(serde_json::Value::as_str) {
            return Some(value.to_string());
        }
    }
    payload
        .get("attributes")
        .and_then(|attributes| {
            ["toolCallId", "tool_call_id", "tool_id", "id"]
                .iter()
                .find_map(|key| attributes.get(key).and_then(serde_json::Value::as_str))
        })
        .map(str::to_string)
}

fn agent_tool_event_title(event: &LaneEventRecord) -> Option<String> {
    let payload = event.payload.as_ref()?;
    for key in ["title", "name", "tool", "command"] {
        if let Some(value) = payload.get(key).and_then(serde_json::Value::as_str) {
            return Some(value.to_string());
        }
    }
    payload
        .get("attributes")
        .and_then(|attributes| {
            ["title", "name", "tool", "command"]
                .iter()
                .find_map(|key| attributes.get(key).and_then(serde_json::Value::as_str))
        })
        .map(str::to_string)
}

fn agent_tool_event_kind(event: &LaneEventRecord) -> Option<String> {
    let payload = event.payload.as_ref()?;
    payload
        .get("kind")
        .or_else(|| payload.get("span_type"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            payload
                .get("attributes")
                .and_then(|attributes| attributes.get("kind").or_else(|| attributes.get("type")))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_string)
}

fn agent_tool_event_status(event: &LaneEventRecord) -> Option<String> {
    let payload = event.payload.as_ref()?;
    payload
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn agent_tools_summary(
    task: &AgentTaskReport,
    total_tool_events: usize,
    unique_tools: usize,
    turns_with_tools: usize,
    available_commands: usize,
) -> String {
    if total_tool_events == 0 && available_commands == 0 {
        return format!(
            "{} has no captured tool activity yet.",
            agent_task_label(task)
        );
    }
    let tool_word = if unique_tools == 1 { "tool" } else { "tools" };
    let event_word = if total_tool_events == 1 {
        "event"
    } else {
        "events"
    };
    let turn_word = if turns_with_tools == 1 {
        "turn"
    } else {
        "turns"
    };
    format!(
        "{} captured {total_tool_events} tool {event_word} across {unique_tools} {tool_word} and {turns_with_tools} {turn_word}; {available_commands} available command(s) were advertised.",
        agent_task_label(task)
    )
}

fn agent_tools_suggestions(lane: &str, tools: &[AgentToolEntry]) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![
        StatusSuggestion {
            command: format!("crabdb agent changes {lane}"),
            reason: "connect tool activity to prompt-to-checkpoint changes".to_string(),
        },
        StatusSuggestion {
            command: format!("crabdb agent timeline {lane}"),
            reason: "see tools, prompts, checkpoints, and changed files in order".to_string(),
        },
    ];
    if let Some(tool) = tools.first().and_then(|tool| tool.turns.first()) {
        suggestions.push(StatusSuggestion {
            command: tool.turn_command.clone(),
            reason: "inspect the turn with the most captured tool activity".to_string(),
        });
        if let Some(command) = &tool.diff_command {
            suggestions.push(StatusSuggestion {
                command: command.clone(),
                reason: "inspect the patch from that tool-heavy turn".to_string(),
            });
        }
    }
    suggestions
}

fn agent_impact_areas(lane: &str, changed_paths: &[FileDiffSummary]) -> Vec<AgentImpactArea> {
    let mut areas: BTreeMap<String, AgentImpactArea> = BTreeMap::new();
    for change in changed_paths {
        let profile = agent_impact_profile(&change.path);
        let area = areas
            .entry(profile.key.to_string())
            .or_insert_with(|| AgentImpactArea {
                key: profile.key.to_string(),
                label: profile.label.to_string(),
                severity: profile.severity.to_string(),
                changed_paths: Vec::new(),
                changed_lines: 0,
                reasons: Vec::new(),
                review_command: format!("crabdb agent changes {lane} --by-file"),
                diff_command: None,
            });
        if agent_impact_severity_rank(profile.severity) > agent_impact_severity_rank(&area.severity)
        {
            area.severity = profile.severity.to_string();
        }
        area.changed_lines += change.additions + change.deletions;
        area.changed_paths.push(change.clone());
        agent_push_unique_string(&mut area.reasons, profile.reason.to_string());
    }
    let mut areas = areas.into_values().collect::<Vec<_>>();
    for area in &mut areas {
        area.changed_paths
            .sort_by(|left, right| left.path.cmp(&right.path));
        if area.changed_paths.len() == 1 {
            let path = &area.changed_paths[0].path;
            area.review_command = format!("crabdb agent file {lane} {}", agent_shell_arg(path));
            area.diff_command = Some(format!(
                "crabdb agent file {lane} {} --patch",
                agent_shell_arg(path)
            ));
        } else if let Some(first) = area.changed_paths.first() {
            area.diff_command = Some(format!(
                "crabdb agent file {lane} {} --patch",
                agent_shell_arg(&first.path)
            ));
        }
    }
    areas
}

struct AgentImpactProfile {
    key: &'static str,
    label: &'static str,
    severity: &'static str,
    reason: &'static str,
}

fn agent_impact_profile(path: &str) -> AgentImpactProfile {
    let lower = path.to_ascii_lowercase();
    let filename = lower.rsplit('/').next().unwrap_or(lower.as_str());
    if matches!(
        filename,
        "cargo.lock"
            | "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "bun.lock"
            | "bun.lockb"
            | "go.sum"
            | "gemfile.lock"
            | "poetry.lock"
    ) {
        return AgentImpactProfile {
            key: "dependencies",
            label: "Dependencies",
            severity: "high",
            reason: "lockfile or dependency resolution changed",
        };
    }
    if matches!(
        filename,
        "cargo.toml"
            | "package.json"
            | "pyproject.toml"
            | "go.mod"
            | "makefile"
            | "justfile"
            | "dockerfile"
    ) || lower.starts_with(".github/workflows/")
        || lower.contains("docker-compose")
    {
        return AgentImpactProfile {
            key: "build_config",
            label: "Build and Project Config",
            severity: "high",
            reason: "build, package, CI, or project manifest changed",
        };
    }
    if lower.ends_with("src/lib.rs")
        || lower.contains("/api/")
        || lower.contains("/schema")
        || lower.contains("/schemas/")
        || lower.contains("/proto/")
        || lower.contains("openapi")
        || lower.contains("/types/")
    {
        return AgentImpactProfile {
            key: "public_api",
            label: "Public API Surface",
            severity: "high",
            reason: "public API, schema, protocol, or exported type surface changed",
        };
    }
    if lower.contains("/mcp/")
        || lower.contains("/acp")
        || lower.contains("/server/")
        || lower.contains("/http")
        || lower.contains("/integrations/")
    {
        return AgentImpactProfile {
            key: "integrations",
            label: "Integrations and Protocols",
            severity: "high",
            reason: "integration, protocol, or server-facing code changed",
        };
    }
    if lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.ends_with("_test.rs")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".spec.ts")
        || lower.ends_with(".test.js")
        || lower.ends_with(".spec.js")
    {
        return AgentImpactProfile {
            key: "tests",
            label: "Tests",
            severity: "medium",
            reason: "test coverage or fixtures changed",
        };
    }
    if filename == "readme.md"
        || lower.starts_with("docs/")
        || lower.ends_with(".md")
        || lower.ends_with(".mdx")
        || lower.ends_with(".rst")
    {
        return AgentImpactProfile {
            key: "docs",
            label: "Documentation",
            severity: "low",
            reason: "documentation or prose changed",
        };
    }
    if lower.contains("/cli/")
        || lower.contains("/command/")
        || lower.contains("/render/")
        || lower.ends_with(".css")
        || lower.ends_with(".html")
        || lower.ends_with(".tsx")
        || lower.ends_with(".jsx")
    {
        return AgentImpactProfile {
            key: "cli_ui",
            label: "CLI or User Interface",
            severity: "medium",
            reason: "user-facing command or interface code changed",
        };
    }
    if lower.ends_with(".toml")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".json")
        || lower.ends_with(".ini")
        || filename.starts_with(".env")
    {
        return AgentImpactProfile {
            key: "configuration",
            label: "Configuration",
            severity: "medium",
            reason: "configuration data changed",
        };
    }
    if lower.ends_with(".rs")
        || lower.ends_with(".go")
        || lower.ends_with(".py")
        || lower.ends_with(".ts")
        || lower.ends_with(".js")
        || lower.ends_with(".java")
        || lower.ends_with(".kt")
        || lower.ends_with(".swift")
    {
        return AgentImpactProfile {
            key: "core_logic",
            label: "Core Code",
            severity: "medium",
            reason: "runtime source code changed",
        };
    }
    AgentImpactProfile {
        key: "other",
        label: "Other Files",
        severity: "low",
        reason: "changed file does not match a known high-signal area",
    }
}

fn agent_impact_severity_rank(severity: &str) -> u8 {
    match severity {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn agent_impact_recommendations(
    lane: &str,
    areas: &[AgentImpactArea],
    risk: &AgentRiskReport,
    validation: &AgentValidationReport,
    changed_paths: &[FileDiffSummary],
) -> Vec<StatusSuggestion> {
    let mut recommendations = Vec::new();
    if let Some(area) = areas.first() {
        agent_push_suggestion(
            &mut recommendations,
            area.review_command.clone(),
            "start review in the highest-impact changed area",
        );
    }
    if validation.needs_test || validation.needs_eval {
        agent_push_suggestion(
            &mut recommendations,
            validation.next.command.clone(),
            &validation.next.reason,
        );
    }
    if agent_changed_manifest_or_api_surface(changed_paths) {
        for suggestion in &validation.suggestions {
            if suggestion.reason.contains("broader test gate") {
                agent_push_suggestion(
                    &mut recommendations,
                    suggestion.command.clone(),
                    &suggestion.reason,
                );
            }
        }
    }
    if matches!(
        risk.level,
        AgentRiskLevel::Medium | AgentRiskLevel::High | AgentRiskLevel::Blocking
    ) {
        agent_push_suggestion(
            &mut recommendations,
            format!("crabdb agent risk {lane}"),
            "inspect risk reasons and mitigation steps before applying",
        );
    }
    agent_push_suggestion(
        &mut recommendations,
        format!("crabdb agent confidence {lane}"),
        "finish with one go/no-go verdict after review and validation",
    );
    recommendations
}

fn agent_impact_suggestions(
    lane: &str,
    recommendations: &[StatusSuggestion],
) -> Vec<StatusSuggestion> {
    let mut suggestions = recommendations.to_vec();
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane} --by-file"),
        "inspect every changed file with prompt and checkpoint provenance",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent review-flow {lane}"),
        "walk through inspect, mark reviewed, validate, and finish",
    );
    suggestions
}

fn agent_impact_summary(
    task: &AgentTaskReport,
    areas: &[AgentImpactArea],
    changed_lines: u64,
    highest_impact: &str,
    validation: &AgentValidationReport,
) -> String {
    if areas.is_empty() {
        return format!(
            "{} has no recorded changed files yet.",
            agent_task_label(task)
        );
    }
    let top_areas = areas
        .iter()
        .take(3)
        .map(|area| area.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} touched {} file(s) and {changed_lines} changed line(s) across {top_areas}; highest impact `{highest_impact}`, validation `{}`.",
        agent_task_label(task),
        task.changed_paths.len(),
        validation.status
    )
}

fn agent_review_map_areas(
    lane: &str,
    workdir: Option<&str>,
    changed_paths: &[FileDiffSummary],
    priorities: &[AgentReviewPriority],
    review_status: &str,
    reviewed_files: &BTreeMap<String, AgentFileReviewMarker>,
) -> Vec<AgentReviewMapArea> {
    let priorities_by_path = priorities
        .iter()
        .map(|priority| (priority.change.path.as_str(), priority))
        .collect::<BTreeMap<_, _>>();
    let mut areas = agent_impact_areas(lane, changed_paths)
        .into_iter()
        .map(|area| {
            let mut files = area
                .changed_paths
                .iter()
                .filter_map(|change| {
                    priorities_by_path
                        .get(change.path.as_str())
                        .map(|priority| {
                            let file_arg = agent_shell_arg(&change.path);
                            let reviewed = if review_status == "up_to_date" {
                                None
                            } else {
                                reviewed_files.get(&change.path).cloned()
                            };
                            let state = if review_status == "up_to_date" || reviewed.is_some() {
                                "reviewed"
                            } else {
                                "needs_review"
                            };
                            let open_path = workdir.map(|workdir| {
                                Path::new(workdir)
                                    .join(&change.path)
                                    .to_string_lossy()
                                    .to_string()
                            });
                            let open_command = open_path
                                .as_ref()
                                .map(|path| format!("${{EDITOR:-vi}} {}", shell_quote(path)));
                            AgentReviewMapFile {
                                rank: priority.rank,
                                path: change.path.clone(),
                                state: state.to_string(),
                                reviewed,
                                change: change.clone(),
                                score: priority.score,
                                reasons: priority.reasons.clone(),
                                touched_by: priority.touched_by.clone(),
                                review_command: format!(
                                    "crabdb agent focus {lane} --file {file_arg}"
                                ),
                                why_command: priority.why_command.clone(),
                                diff_command: priority.diff_command.clone(),
                                open_path,
                                open_command,
                            }
                        })
                })
                .collect::<Vec<_>>();
            files.sort_by(|left, right| {
                left.rank
                    .cmp(&right.rank)
                    .then_with(|| left.path.cmp(&right.path))
            });
            let state = if files.is_empty() {
                "empty"
            } else if files.iter().all(|file| file.state == "reviewed") {
                "reviewed"
            } else {
                "needs_review"
            }
            .to_string();
            let review_command = files
                .first()
                .map(|file| file.review_command.clone())
                .unwrap_or(area.review_command.clone());
            let patch_command = files
                .first()
                .and_then(|file| file.diff_command.clone())
                .or(area.diff_command.clone());
            AgentReviewMapArea {
                key: area.key,
                label: area.label,
                severity: area.severity,
                state,
                changed_paths: area.changed_paths,
                changed_lines: area.changed_lines,
                reasons: area.reasons,
                files,
                review_command,
                patch_command,
            }
        })
        .collect::<Vec<_>>();
    areas.retain(|area| !area.changed_paths.is_empty());
    areas
}

fn agent_review_map_next(
    lane: &str,
    areas: &[AgentReviewMapArea],
    progress: &AgentReviewProgress,
    validation: &AgentValidationReport,
) -> StatusSuggestion {
    if let Some(file) = areas
        .iter()
        .flat_map(|area| area.files.iter())
        .find(|file| file.state != "reviewed")
    {
        return StatusSuggestion {
            command: file.review_command.clone(),
            reason: "start with the highest-priority file that still needs review".to_string(),
        };
    }
    if progress.status != "up_to_date" {
        return StatusSuggestion {
            command: format!("crabdb agent new {lane}"),
            reason: "inspect changes since the latest reviewed checkpoint".to_string(),
        };
    }
    if validation.needs_test || validation.needs_eval {
        return validation.next.clone();
    }
    StatusSuggestion {
        command: format!("crabdb agent confidence {lane}"),
        reason: "finish with one go/no-go verdict before applying".to_string(),
    }
}

fn agent_review_map_suggestions(
    lane: &str,
    next: &StatusSuggestion,
    areas: &[AgentReviewMapArea],
    validation: &AgentValidationReport,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent impact {lane}"),
        "review the blast radius and impacted areas behind this map",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane} --by-file"),
        "inspect every changed file with prompt and checkpoint provenance",
    );
    if let Some(area) = areas.first() {
        agent_push_suggestion(
            &mut suggestions,
            area.review_command.clone(),
            "start in the highest-impact review area",
        );
        if let Some(command) = &area.patch_command {
            agent_push_suggestion(
                &mut suggestions,
                command.clone(),
                "inspect the first focused patch from the highest-impact area",
            );
        }
    }
    if validation.needs_test || validation.needs_eval {
        agent_push_suggestion(
            &mut suggestions,
            validation.next.command.clone(),
            &validation.next.reason,
        );
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent review-flow {lane}"),
        "walk through inspect, mark reviewed, validate, and finish",
    );
    suggestions
}

fn agent_review_map_summary(
    task: &AgentTaskReport,
    areas: &[AgentReviewMapArea],
    changed_lines: u64,
    progress: &AgentReviewProgress,
    validation: &AgentValidationReport,
) -> String {
    if areas.is_empty() {
        return format!("{} has no changed files to review.", agent_task_label(task));
    }
    let top_area = areas
        .first()
        .map(|area| area.label.as_str())
        .unwrap_or("changed files");
    let top_file = areas
        .iter()
        .flat_map(|area| area.files.iter())
        .next()
        .map(|file| file.path.as_str())
        .unwrap_or("the first changed file");
    format!(
        "{} has {} changed file(s) and {changed_lines} changed line(s) across {} review area(s). Review status `{}`, validation `{}`. Start with {top_area}: `{top_file}`.",
        agent_task_label(task),
        task.changed_paths.len(),
        areas.len(),
        progress.status,
        validation.status
    )
}

fn agent_risk_report_from_view(view: &AgentTaskViewReport) -> AgentRiskReport {
    let mut score: u8 = 0;
    let mut reasons = Vec::new();
    let mut recommendations = Vec::new();
    let lane = &view.task.lane;

    if view.task.status == AgentTaskStatus::Applied {
        return AgentRiskReport {
            task: view.task.clone(),
            level: AgentRiskLevel::Low,
            score: 0,
            summary:
                "Task has already been applied; use `crabdb agent continue` for follow-up work."
                    .to_string(),
            reasons,
            recommendations: agent_already_applied_suggestions(&view.task),
        };
    }

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
            command: format!("crabdb agent finish {lane}"),
            reason: "hide the applied task from the default inbox when you are done".to_string(),
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
    if view.task.status == AgentTaskStatus::Applied {
        return suggestions;
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

fn agent_confidence_factors(
    lane: &str,
    progress: &AgentReviewProgress,
    validation: &AgentValidationReport,
    ready: &AgentReadyReport,
    risk: &AgentRiskReport,
) -> Vec<AgentConfidenceFactor> {
    let mut factors = Vec::new();

    let (review_state, review_delta, review_message, review_command) =
        match progress.status.as_str() {
            "up_to_date" => (
                "pass",
                0,
                "current checkpoint has been marked reviewed".to_string(),
                Some(format!("crabdb agent review-flow {lane}")),
            ),
            "new_changes" => (
                "warn",
                -18,
                format!(
                    "{} changed file(s) and {} changed line(s) have not been reviewed since the last marker",
                    progress.changed_paths, progress.changed_lines
                ),
                Some(format!("crabdb agent review-flow {lane}")),
            ),
            _ => (
                "warn",
                -22,
                format!(
                    "{} changed file(s) and {} changed line(s) have not been marked reviewed yet",
                    progress.changed_paths, progress.changed_lines
                ),
                Some(format!("crabdb agent review-flow {lane}")),
            ),
        };
    factors.push(agent_confidence_factor(
        "review",
        review_state,
        review_delta,
        review_message,
        review_command,
    ));

    let (validation_state, validation_delta, validation_message, validation_command) =
        if validation.needs_test && validation.needs_eval {
            (
                "warn",
                -18,
                "latest test and eval gates are both missing".to_string(),
                Some(validation.next.command.clone()),
            )
        } else if validation.needs_test {
            (
                "warn",
                -14,
                "latest test gate is missing".to_string(),
                Some(validation.next.command.clone()),
            )
        } else if validation.needs_eval {
            (
                "warn",
                -8,
                "latest eval gate is missing".to_string(),
                Some(validation.next.command.clone()),
            )
        } else {
            (
                "pass",
                0,
                "validation guidance has no required missing gate".to_string(),
                Some(format!("crabdb agent validate {lane}")),
            )
        };
    factors.push(agent_confidence_factor(
        "validation",
        validation_state,
        validation_delta,
        validation_message,
        validation_command,
    ));

    let (risk_state, risk_delta, risk_message) = match risk.level {
        AgentRiskLevel::Low => ("pass", 0, format!("risk is low ({}/100)", risk.score)),
        AgentRiskLevel::Medium => ("warn", -12, format!("risk is medium ({}/100)", risk.score)),
        AgentRiskLevel::High => ("warn", -25, format!("risk is high ({}/100)", risk.score)),
        AgentRiskLevel::Blocking => (
            "block",
            -45,
            format!("risk is blocking ({}/100)", risk.score),
        ),
    };
    factors.push(agent_confidence_factor(
        "risk",
        risk_state,
        risk_delta,
        risk_message,
        risk.recommendations
            .first()
            .map(|suggestion| suggestion.command.clone()),
    ));

    let (apply_state, apply_delta, apply_message, apply_command) = if ready.ready {
        (
            "pass",
            0,
            "Git dry-run preflight and CrabDB readiness passed".to_string(),
            Some(format!("crabdb agent finish {lane} --dry-run")),
        )
    } else {
        match ready.status.as_str() {
            "applied" => (
                "pass",
                0,
                "task has already been applied".to_string(),
                Some(format!("crabdb agent finish {lane}")),
            ),
            "needs_record" => (
                "warn",
                -20,
                "materialized task workdir has unrecorded changes".to_string(),
                Some(ready.next.command.clone()),
            ),
            "conflicted" | "git_blocked" => (
                "block",
                -45,
                ready.summary.clone(),
                Some(ready.next.command.clone()),
            ),
            _ => (
                if ready.blockers.is_empty() {
                    "warn"
                } else {
                    "block"
                },
                if ready.blockers.is_empty() { -18 } else { -40 },
                ready.summary.clone(),
                Some(ready.next.command.clone()),
            ),
        }
    };
    factors.push(agent_confidence_factor(
        "apply_preflight",
        apply_state,
        apply_delta,
        apply_message,
        apply_command,
    ));

    factors
}

fn agent_confidence_factor(
    name: &str,
    state: &str,
    score_delta: i16,
    message: String,
    command: Option<String>,
) -> AgentConfidenceFactor {
    AgentConfidenceFactor {
        name: name.to_string(),
        state: state.to_string(),
        score_delta,
        message,
        command,
    }
}

fn agent_confidence_score(factors: &[AgentConfidenceFactor], risk: &AgentRiskReport) -> u8 {
    let risk_penalty = match risk.level {
        AgentRiskLevel::Low => 0,
        AgentRiskLevel::Medium => 5,
        AgentRiskLevel::High => 15,
        AgentRiskLevel::Blocking => 30,
    };
    let factor_delta = factors.iter().map(|factor| factor.score_delta).sum::<i16>();
    let score = 100i16 - risk_penalty + factor_delta;
    score.clamp(0, 100) as u8
}

fn agent_confidence_verdict(
    task: &AgentTaskReport,
    progress: &AgentReviewProgress,
    validation: &AgentValidationReport,
    ready: &AgentReadyReport,
    risk: &AgentRiskReport,
    score: u8,
) -> String {
    if task.status == AgentTaskStatus::Applied || ready.status == "applied" {
        return "applied".to_string();
    }
    if progress.status != "up_to_date" {
        return "review".to_string();
    }
    if validation.needs_test || validation.needs_eval {
        return "validate".to_string();
    }
    if ready.status == "conflicted" || ready.status == "git_blocked" || !ready.blockers.is_empty() {
        return "blocked".to_string();
    }
    if !ready.ready {
        return "wait".to_string();
    }
    if matches!(risk.level, AgentRiskLevel::Blocking | AgentRiskLevel::High) || score < 70 {
        return "review".to_string();
    }
    "go".to_string()
}

fn agent_confidence_next(
    lane: &str,
    verdict: &str,
    progress: &AgentReviewProgress,
    validation: &AgentValidationReport,
    ready: &AgentReadyReport,
) -> StatusSuggestion {
    match verdict {
        "applied" => StatusSuggestion {
            command: format!("crabdb agent finish {lane}"),
            reason: "hide the already-applied task from the default inbox when done".to_string(),
        },
        "blocked" | "wait" => ready.next.clone(),
        "review" if progress.status != "up_to_date" => StatusSuggestion {
            command: format!("crabdb agent review-flow {lane}"),
            reason: "walk through review before deciding whether to apply".to_string(),
        },
        "review" => StatusSuggestion {
            command: format!("crabdb agent review-plan {lane}"),
            reason: "inspect higher-risk files before applying".to_string(),
        },
        "validate" => validation.next.clone(),
        "go" => StatusSuggestion {
            command: format!("crabdb agent finish {lane} --dry-run"),
            reason: "preview apply and archive before mutating Git".to_string(),
        },
        _ => ready.next.clone(),
    }
}

fn agent_confidence_suggestions(
    lane: &str,
    primary: &StatusSuggestion,
    verdict: &str,
    validation: &AgentValidationReport,
    ready: &AgentReadyReport,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![primary.clone()];
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent review-flow {lane}"),
        "walk through inspect, mark reviewed, validate, and finish",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent changes {lane} --by-file"),
        "review changed files with prompt and checkpoint provenance",
    );
    agent_push_suggestion(
        &mut suggestions,
        validation.next.command.clone(),
        &validation.next.reason,
    );
    agent_push_suggestion(
        &mut suggestions,
        ready.next.command.clone(),
        &ready.next.reason,
    );
    if verdict == "go" {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent finish {lane} --dry-run"),
            "preview the final apply and cleanup plan",
        );
    }
    suggestions
}

fn agent_confidence_summary(
    task: &AgentTaskReport,
    verdict: &str,
    score: u8,
    progress: &AgentReviewProgress,
    validation: &AgentValidationReport,
    ready: &AgentReadyReport,
) -> String {
    let label = agent_task_label(task);
    let validation_state = if validation.needs_test || validation.needs_eval {
        validation.status.as_str()
    } else {
        "current"
    };
    format!(
        "{label}: verdict `{verdict}`, confidence {score}/100, review `{}`, validation `{validation_state}`, apply `{}`.",
        progress.status, ready.status
    )
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

fn agent_dashboard_summary(
    task: &AgentTaskReport,
    ready: &AgentReadyReport,
    validation: &AgentValidationReport,
    focus: Option<&AgentFocusReport>,
) -> String {
    let title = agent_task_label(task);
    let focus_text = focus
        .map(|report| format!("next file `{}`", report.path))
        .unwrap_or_else(|| "no changed file focus".to_string());
    format!(
        "{title}: {:?}, {} changed file(s), {focus_text}, validation `{}`, apply `{}`.",
        task.status,
        task.changed_paths.len(),
        validation.status,
        ready.status
    )
}

fn agent_review_data_summary(
    task: &AgentTaskReport,
    confidence: &AgentConfidenceReport,
    total_files: usize,
    reviewed_files: usize,
    needs_review_files: usize,
) -> String {
    let title = agent_task_label(task);
    let progress = if total_files == 0 {
        "no changed files".to_string()
    } else if needs_review_files == 0 {
        format!("all {total_files} changed file(s) reviewed")
    } else {
        format!("{reviewed_files}/{total_files} changed file(s) reviewed")
    };
    format!(
        "{title}: {progress}, verdict `{}`, confidence {}/100, validation `{}`, apply `{}`.",
        confidence.verdict, confidence.score, confidence.validation.status, confidence.ready.status
    )
}

fn agent_review_data_actions(
    lane: &str,
    focus: Option<&AgentFocusReport>,
    confidence: &AgentConfidenceReport,
    needs_review_files: usize,
) -> Vec<AgentReviewAction> {
    let mut actions = Vec::new();
    if let Some(focus) = focus {
        let path_arg = agent_shell_arg(&focus.path);
        let open_command = focus
            .open_command
            .clone()
            .unwrap_or_else(|| format!("crabdb agent open {lane} --file {path_arg}"));
        actions.push(agent_review_action(
            "open_focus_file",
            "Open focus file",
            "open_file",
            open_command,
            "open the highest-priority changed file",
            true,
            None,
            "read_only",
            false,
            Some(focus.path.clone()),
            focus.open_path.clone(),
            None,
            None,
        ));
        actions.push(agent_review_action(
            "inspect_focus_file",
            "Inspect focus file",
            "inspect_file",
            format!("crabdb agent focus {lane} --file {path_arg}"),
            "show why this file is the next review target",
            true,
            None,
            "read_only",
            false,
            Some(focus.path.clone()),
            focus.open_path.clone(),
            Some("crabdb.agent_focus"),
            Some(serde_json::json!({
                "selector": lane,
                "file": focus.path,
                "patch": false
            })),
        ));
        actions.push(agent_review_action(
            "show_focus_patch",
            "Show focus patch",
            "show_patch",
            format!("crabdb agent focus {lane} --file {path_arg} --patch"),
            "show the focused diff for this file",
            true,
            None,
            "read_only",
            false,
            Some(focus.path.clone()),
            focus.open_path.clone(),
            Some("crabdb.agent_focus"),
            Some(serde_json::json!({
                "selector": lane,
                "file": focus.path,
                "patch": true
            })),
        ));
        actions.push(agent_review_action(
            "mark_focus_file_reviewed",
            "Mark file reviewed",
            "mark_file_reviewed",
            format!("crabdb agent done-file {lane} {path_arg}"),
            "record that this changed file has been reviewed",
            needs_review_files > 0,
            if needs_review_files > 0 {
                None
            } else {
                Some("all changed files are already reviewed".to_string())
            },
            "workspace_write",
            false,
            Some(focus.path.clone()),
            focus.open_path.clone(),
            Some("crabdb.agent_mark_file_reviewed"),
            Some(serde_json::json!({
                "selector": lane,
                "path": focus.path,
                "note": null
            })),
        ));
    }

    actions.push(agent_review_action(
        "show_review_map",
        "Show review map",
        "show_review_map",
        format!("crabdb agent review-map {lane}"),
        "show file-level review progress grouped by area",
        true,
        None,
        "read_only",
        false,
        None,
        None,
        Some("crabdb.agent_review_map"),
        Some(serde_json::json!({ "selector": lane })),
    ));
    actions.push(agent_review_action(
        "show_test_plan",
        "Show test plan",
        "show_test_plan",
        format!("crabdb agent test-plan {lane}"),
        "show exact validation commands before applying",
        true,
        None,
        "read_only",
        false,
        None,
        None,
        Some("crabdb.agent_test_plan"),
        Some(serde_json::json!({ "selector": lane })),
    ));

    let validation_safety = agent_review_action_safety(&confidence.validation.next.command);
    actions.push(agent_review_action(
        "validation_next",
        "Run validation next step",
        "validation_next",
        confidence.validation.next.command.clone(),
        confidence.validation.next.reason.clone(),
        true,
        None,
        validation_safety,
        validation_safety != "read_only",
        None,
        None,
        agent_review_action_mcp_tool(&confidence.validation.next.command),
        agent_review_action_mcp_arguments(
            &confidence.validation.next.command,
            agent_review_action_mcp_tool(&confidence.validation.next.command),
            lane,
        ),
    ));
    actions.push(agent_review_action(
        "apply_dry_run",
        "Preview apply",
        "apply_dry_run",
        format!("crabdb agent apply {lane} --dry-run"),
        "preview applying this task without mutating Git",
        true,
        None,
        "read_only",
        false,
        None,
        None,
        Some("crabdb.agent_apply"),
        Some(serde_json::json!({
            "selector": lane,
            "dry-run": true,
            "message": null
        })),
    ));
    let apply_enabled = confidence.ready.ready;
    actions.push(agent_review_action(
        "apply_task",
        "Apply task",
        "apply",
        format!("crabdb agent finish {lane}"),
        if confidence.ready.ready {
            "apply this task and hide it from the default inbox"
        } else {
            "disabled until review, validation, risk, and apply preflight are ready"
        },
        apply_enabled,
        if apply_enabled {
            None
        } else {
            Some("review, validation, risk, or apply preflight is not ready".to_string())
        },
        "destructive",
        true,
        None,
        None,
        Some("crabdb.agent_finish"),
        Some(serde_json::json!({
            "selector": lane,
            "dry-run": false,
            "message": null,
            "note": null
        })),
    ));
    if needs_review_files == 0 && confidence.review_status != "up_to_date" {
        actions.push(agent_review_action(
            "mark_task_reviewed",
            "Mark task reviewed",
            "mark_task_reviewed",
            format!("crabdb agent done {lane}"),
            "record the current checkpoint as reviewed after all files are inspected",
            true,
            None,
            "workspace_write",
            false,
            None,
            None,
            Some("crabdb.agent_mark_reviewed"),
            Some(serde_json::json!({
                "selector": lane,
                "note": null
            })),
        ));
    }
    actions
}

#[allow(clippy::too_many_arguments)]
fn agent_review_action(
    id: &str,
    label: &str,
    kind: &str,
    command: impl Into<String>,
    reason: impl Into<String>,
    enabled: bool,
    disabled_reason: Option<String>,
    safety: &str,
    requires_confirmation: bool,
    path: Option<String>,
    open_path: Option<String>,
    mcp_tool: Option<&str>,
    mcp_arguments: Option<serde_json::Value>,
) -> AgentReviewAction {
    AgentReviewAction {
        id: id.to_string(),
        label: label.to_string(),
        kind: kind.to_string(),
        command: command.into(),
        reason: reason.into(),
        enabled,
        disabled_reason,
        safety: safety.to_string(),
        requires_confirmation,
        path,
        open_path,
        mcp_tool: mcp_tool.map(ToString::to_string),
        mcp_arguments,
    }
}

fn agent_review_action_safety(command: &str) -> &'static str {
    if command.contains(" agent test ") || command.contains(" agent eval ") {
        "open_world"
    } else if command.contains(" done")
        || command.contains("mark-reviewed")
        || command.contains("mark-file-reviewed")
    {
        "workspace_write"
    } else if command.contains(" finish ")
        || command.contains(" apply ")
        || command.contains(" rewind")
        || command.contains(" undo")
    {
        if command.contains("--dry-run") {
            "read_only"
        } else {
            "destructive"
        }
    } else {
        "read_only"
    }
}

fn agent_review_action_mcp_tool(command: &str) -> Option<&'static str> {
    if command.contains(" agent test ") {
        Some("crabdb.agent_test")
    } else if command.contains(" agent eval ") {
        Some("crabdb.agent_eval")
    } else if command.contains(" agent ready ") || command.contains(" agent can-land ") {
        Some("crabdb.agent_ready")
    } else if command.contains(" agent test-plan ") {
        Some("crabdb.agent_test_plan")
    } else {
        None
    }
}

fn agent_review_action_mcp_arguments(
    command: &str,
    tool: Option<&str>,
    lane: &str,
) -> Option<serde_json::Value> {
    match tool {
        Some("crabdb.agent_ready") => Some(serde_json::json!({ "selector": lane })),
        Some("crabdb.agent_test_plan") => Some(serde_json::json!({ "selector": lane })),
        Some("crabdb.agent_test") | Some("crabdb.agent_eval") => {
            agent_review_action_gate_args(command, lane)
        }
        _ => None,
    }
}

fn agent_review_action_gate_args(command: &str, lane: &str) -> Option<serde_json::Value> {
    let marker = if command.contains(" agent test ") {
        " -- "
    } else if command.contains(" agent eval ") {
        " -- "
    } else {
        return None;
    };
    let (_prefix, tail) = command.split_once(marker)?;
    let command_args = tail
        .split_whitespace()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>();
    if command_args.is_empty() || command_args.iter().any(|part| part.starts_with('<')) {
        return None;
    }
    Some(serde_json::json!({
        "selector": lane,
        "command": command_args
    }))
}

fn agent_review_data_suggestions(
    lane: &str,
    next: &StatusSuggestion,
    focus: Option<&AgentFocusReport>,
    needs_review_files: usize,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    if let Some(focus) = focus {
        agent_push_suggestion(
            &mut suggestions,
            format!(
                "crabdb agent focus {lane} --file {}",
                agent_shell_arg(&focus.path)
            ),
            "inspect the current highest-priority file with provenance",
        );
        if needs_review_files > 0 {
            agent_push_suggestion(
                &mut suggestions,
                format!(
                    "crabdb agent done-file {lane} {}",
                    agent_shell_arg(&focus.path)
                ),
                "mark this file reviewed after inspection",
            );
        }
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent review-map {lane}"),
        "see file-level review progress grouped by area",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent test-plan {lane}"),
        "see exact validation commands to run before applying",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent can-land {lane}"),
        "check apply readiness without mutating Git",
    );
    suggestions
}

fn agent_already_applied_suggestions(task: &AgentTaskReport) -> Vec<StatusSuggestion> {
    let provider = task.provider.as_deref().unwrap_or("claude-code");
    let mut suggestions = Vec::new();
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent view {}", task.lane),
        "inspect the applied task transcript, tools, and checkpoint",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent finish {}", task.lane),
        "hide the applied task from the default inbox when you are done",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent continue {} --provider {provider}", task.lane),
        "start a fresh follow-up task from this applied checkpoint instead of reusing the applied lane",
    );
    agent_push_suggestion(
        &mut suggestions,
        "crabdb agent inbox".to_string(),
        "choose another active agent task",
    );
    suggestions
}

fn agent_finish_suggestions(
    task: &AgentTaskReport,
    apply: &AgentApplyReport,
    status: &str,
    dry_run: bool,
) -> Vec<StatusSuggestion> {
    let lane = &task.lane;
    let mut suggestions = Vec::new();
    if dry_run && matches!(status, "ready") {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent finish {lane}"),
            "apply the task and hide it from the default inbox after success",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent land {lane}"),
            "apply the task but keep it visible in the default inbox",
        );
        return suggestions;
    }
    if status == "finished" {
        agent_push_suggestion(
            &mut suggestions,
            "crabdb agent inbox".to_string(),
            "pick the next active agent task",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent receipt {lane}"),
            "print the applied task receipt if you need to share it",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent continue {lane}"),
            "start a fresh follow-up task from this applied checkpoint",
        );
        return suggestions;
    }
    for suggestion in &apply.suggestions {
        agent_push_suggestion(
            &mut suggestions,
            suggestion.command.clone(),
            &suggestion.reason,
        );
    }
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent view {lane}"),
        "inspect the task transcript, tools, and checkpoint",
    );
    suggestions
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

fn agent_test_plan_steps(
    lane: &str,
    workspace_root: &Path,
    validation: &AgentValidationReport,
    impact: &AgentImpactReport,
) -> Vec<AgentTestPlanStep> {
    let mut steps = Vec::new();
    let high_impact_paths = agent_test_plan_high_impact_paths(&impact.areas);
    let broad_needed = !high_impact_paths.is_empty();
    let test_failed = validation
        .latest_test
        .as_ref()
        .is_some_and(|gate| !gate.success);
    if validation.needs_test {
        let broad = broad_needed;
        let command_parts = agent_validation_default_test_command(workspace_root, broad);
        let paths = if broad_needed {
            high_impact_paths.clone()
        } else {
            validation.changed_paths.clone()
        };
        steps.push(AgentTestPlanStep {
            rank: 0,
            kind: "test".to_string(),
            label: if broad {
                "Run broad test gate".to_string()
            } else {
                "Run test gate".to_string()
            },
            state: if test_failed { "failed" } else { "needed" }.to_string(),
            required: true,
            command: format!(
                "crabdb agent test {lane} -- {}",
                agent_validation_command_text(&command_parts)
            ),
            reason: if test_failed {
                "latest recorded test gate failed; rerun after reviewing the agent changes"
                    .to_string()
            } else if broad {
                "high-impact files changed, so record a broader passing test gate".to_string()
            } else {
                "no passing test gate has been recorded for this task".to_string()
            },
            area_key: if broad {
                Some("high_impact".to_string())
            } else {
                None
            },
            area_label: if broad {
                Some("High-impact changed areas".to_string())
            } else {
                None
            },
            paths,
            latest_gate: validation.latest_test.clone(),
        });
    } else if let Some(gate) = &validation.latest_test {
        steps.push(AgentTestPlanStep {
            rank: 0,
            kind: "test".to_string(),
            label: "Latest test gate passed".to_string(),
            state: "done".to_string(),
            required: false,
            command: format!("crabdb agent validate {lane}"),
            reason: "a passing test gate is already recorded for this task".to_string(),
            area_key: None,
            area_label: None,
            paths: validation.changed_paths.clone(),
            latest_gate: Some(gate.clone()),
        });
    }

    let eval_failed = validation
        .latest_eval
        .as_ref()
        .is_some_and(|gate| !gate.success);
    if validation.needs_eval {
        steps.push(AgentTestPlanStep {
            rank: 0,
            kind: "eval".to_string(),
            label: if eval_failed {
                "Rerun eval gate".to_string()
            } else {
                "Record eval gate when quality policy matters".to_string()
            },
            state: if eval_failed { "failed" } else { "optional" }.to_string(),
            required: eval_failed,
            command: format!("crabdb agent eval {lane} -- <eval-command>"),
            reason: if eval_failed {
                "latest recorded eval gate failed; rerun the relevant quality eval".to_string()
            } else {
                "record this only when model, policy, or product-quality behavior matters"
                    .to_string()
            },
            area_key: None,
            area_label: None,
            paths: validation.changed_paths.clone(),
            latest_gate: validation.latest_eval.clone(),
        });
    } else if let Some(gate) = &validation.latest_eval {
        steps.push(AgentTestPlanStep {
            rank: 0,
            kind: "eval".to_string(),
            label: "Latest eval gate passed".to_string(),
            state: "done".to_string(),
            required: false,
            command: format!("crabdb agent validate {lane}"),
            reason: "a passing eval gate is already recorded for this task".to_string(),
            area_key: None,
            area_label: None,
            paths: validation.changed_paths.clone(),
            latest_gate: Some(gate.clone()),
        });
    }

    if broad_needed && !validation.needs_test {
        let command_parts = agent_validation_default_test_command(workspace_root, true);
        steps.push(AgentTestPlanStep {
            rank: 0,
            kind: "test".to_string(),
            label: "Consider broad regression test".to_string(),
            state: "optional".to_string(),
            required: false,
            command: format!(
                "crabdb agent test {lane} -- {}",
                agent_validation_command_text(&command_parts)
            ),
            reason: "high-impact files changed; run this if the existing gate was too narrow"
                .to_string(),
            area_key: Some("high_impact".to_string()),
            area_label: Some("High-impact changed areas".to_string()),
            paths: high_impact_paths,
            latest_gate: validation.latest_test.clone(),
        });
    }

    steps.push(AgentTestPlanStep {
        rank: 0,
        kind: "readiness".to_string(),
        label: "Check confidence after validation".to_string(),
        state: "pending".to_string(),
        required: false,
        command: format!("crabdb agent confidence {lane}"),
        reason: "finish with one go/no-go verdict after review and validation".to_string(),
        area_key: None,
        area_label: None,
        paths: Vec::new(),
        latest_gate: None,
    });

    for (index, step) in steps.iter_mut().enumerate() {
        step.rank = index + 1;
    }
    steps
}

fn agent_test_plan_high_impact_paths(areas: &[AgentImpactArea]) -> Vec<FileDiffSummary> {
    areas
        .iter()
        .filter(|area| {
            matches!(
                area.key.as_str(),
                "dependencies" | "build_config" | "public_api" | "integrations"
            )
        })
        .flat_map(|area| area.changed_paths.iter().cloned())
        .collect()
}

fn agent_test_plan_status(steps: &[AgentTestPlanStep]) -> String {
    if steps
        .iter()
        .any(|step| step.required && step.state == "failed")
    {
        return "failed_gate".to_string();
    }
    if steps
        .iter()
        .any(|step| step.required && step.state == "needed")
    {
        return "needs_test".to_string();
    }
    if steps
        .iter()
        .any(|step| step.required && step.state != "done")
    {
        return "needs_validation".to_string();
    }
    if steps.iter().any(|step| step.state == "optional") {
        return "optional_eval".to_string();
    }
    "current".to_string()
}

fn agent_test_plan_next(lane: &str, steps: &[AgentTestPlanStep]) -> StatusSuggestion {
    if let Some(step) = steps
        .iter()
        .find(|step| step.required && step.state != "done")
    {
        return StatusSuggestion {
            command: step.command.clone(),
            reason: step.reason.clone(),
        };
    }
    if let Some(step) = steps.iter().find(|step| step.state == "optional") {
        return StatusSuggestion {
            command: step.command.clone(),
            reason: step.reason.clone(),
        };
    }
    StatusSuggestion {
        command: format!("crabdb agent confidence {lane}"),
        reason: "validation is current; finish with one go/no-go verdict".to_string(),
    }
}

fn agent_test_plan_suggestions(
    lane: &str,
    next: &StatusSuggestion,
    validation: &AgentValidationReport,
    impact: &AgentImpactReport,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![next.clone()];
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent validate {lane}"),
        "see latest recorded test and eval gates",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent impact {lane}"),
        "see why the validation plan chose these checks",
    );
    if !impact.areas.is_empty() {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent review-map {lane}"),
            "review changed files before running validation",
        );
    }
    if !validation.recent_gates.is_empty() {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb lane gates {lane}"),
            "inspect recorded validation gate history",
        );
    }
    suggestions
}

fn agent_test_plan_summary(
    task: &AgentTaskReport,
    status: &str,
    steps: &[AgentTestPlanStep],
    impact: &AgentImpactReport,
) -> String {
    let required = steps
        .iter()
        .filter(|step| step.required && step.state != "done")
        .count();
    let optional = steps.iter().filter(|step| step.state == "optional").count();
    let top_area = impact
        .areas
        .first()
        .map(|area| area.label.as_str())
        .unwrap_or("changed files");
    format!(
        "{} validation plan: status `{status}`, {required} required step(s), {optional} optional step(s), highest impact `{}` in {top_area}.",
        agent_task_label(task),
        impact.highest_impact
    )
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

fn agent_review_flow_steps(
    lane: &str,
    new: &AgentNewReport,
    validation: &AgentValidationReport,
    ready: &AgentReadyReport,
    focus: Option<&AgentFocusReport>,
) -> Vec<AgentReviewFlowStep> {
    let review_open = new.status != "up_to_date";
    let validation_open = validation.needs_test || validation.needs_eval;
    let applied = ready.status == "applied" || ready.task.status == AgentTaskStatus::Applied;

    let inspect_command = if review_open {
        new.next.command.clone()
    } else {
        format!("crabdb agent changes {lane}")
    };
    let inspect_reason = if review_open {
        new.next.reason.clone()
    } else if let Some(focus) = focus {
        format!(
            "current checkpoint is reviewed; highest-priority file was `{}`",
            focus.path
        )
    } else {
        "current checkpoint is reviewed and no changed file needs focus".to_string()
    };

    let mut steps = vec![agent_review_flow_step(
        "Inspect changes",
        if review_open { "current" } else { "done" },
        inspect_command,
        inspect_reason,
    )];

    steps.push(agent_review_flow_step(
        "Mark reviewed",
        if review_open { "pending" } else { "done" },
        format!("crabdb agent done {lane}"),
        if review_open {
            "mark the current checkpoint reviewed after inspecting the new changes".to_string()
        } else {
            "current checkpoint is already marked reviewed".to_string()
        },
    ));

    let validation_state = if review_open {
        "pending"
    } else if validation_open {
        "current"
    } else {
        "done"
    };
    let validation_command = if validation_open {
        validation.next.command.clone()
    } else {
        format!("crabdb agent validate {lane}")
    };
    steps.push(agent_review_flow_step(
        "Validate",
        validation_state,
        validation_command,
        if validation_open {
            validation.next.reason.clone()
        } else {
            "latest test/eval guidance has no required gate missing".to_string()
        },
    ));

    let finish_state = if applied {
        "done"
    } else if review_open || validation_open {
        "pending"
    } else if ready.ready || ready.status == "ready" {
        "current"
    } else {
        "blocked"
    };
    let finish_command = if ready.ready || ready.status == "ready" {
        format!("crabdb agent finish {lane} --dry-run")
    } else {
        ready.next.command.clone()
    };
    steps.push(agent_review_flow_step(
        "Finish safely",
        finish_state,
        finish_command,
        if applied {
            "task has already been applied; finish only needs cleanup".to_string()
        } else if review_open {
            "finish waits until new changes are reviewed".to_string()
        } else if validation_open {
            "finish waits until recommended validation is addressed".to_string()
        } else if ready.ready || ready.status == "ready" {
            "preview apply and archive without mutating Git".to_string()
        } else {
            ready.next.reason.clone()
        },
    ));

    steps
}

fn agent_review_flow_step(
    label: &str,
    state: &str,
    command: String,
    reason: String,
) -> AgentReviewFlowStep {
    AgentReviewFlowStep {
        label: label.to_string(),
        state: state.to_string(),
        command,
        reason,
    }
}

fn agent_review_flow_summary(
    task: &AgentTaskReport,
    progress: &AgentReviewProgress,
    validation: &AgentValidationReport,
    ready: &AgentReadyReport,
    focus: Option<&AgentFocusReport>,
) -> String {
    let label = agent_task_label(task);
    let focus_text = focus
        .map(|focus| format!("next focus `{}`", focus.path))
        .unwrap_or_else(|| "no focus file".to_string());
    let validation_text = if validation.needs_test && validation.needs_eval {
        "test and eval missing"
    } else if validation.needs_test {
        "test missing"
    } else if validation.needs_eval {
        "eval missing"
    } else {
        "validation current"
    };
    format!(
        "{label}: review `{}`, {} new file(s), {} new line(s), {focus_text}, {validation_text}, apply `{}`.",
        progress.status, progress.changed_paths, progress.changed_lines, ready.status
    )
}

fn agent_review_flow_suggestions(
    lane: &str,
    primary: &StatusSuggestion,
    review: &AgentReviewReport,
    new: &AgentNewReport,
    validation: &AgentValidationReport,
    ready: &AgentReadyReport,
    focus: Option<&AgentFocusReport>,
) -> Vec<StatusSuggestion> {
    let mut suggestions = vec![primary.clone()];
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent review-flow {lane}"),
        "refresh the guided review checklist after each step",
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent new {lane}"),
        "show what changed since the latest reviewed marker",
    );
    if let Some(focus) = focus {
        agent_push_suggestion(
            &mut suggestions,
            format!(
                "crabdb agent focus {lane} --file {}",
                agent_shell_arg(&focus.path)
            ),
            "inspect the highest-priority file with provenance and diff context",
        );
    } else if let Some(priority) = review.priorities.first() {
        agent_push_suggestion(
            &mut suggestions,
            priority.why_command.clone(),
            "explain why the highest-priority file changed",
        );
    }
    if new.status != "up_to_date" {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent done {lane}"),
            "mark the checkpoint reviewed after inspection",
        );
    }
    agent_push_suggestion(
        &mut suggestions,
        validation.next.command.clone(),
        &validation.next.reason,
    );
    agent_push_suggestion(
        &mut suggestions,
        ready.next.command.clone(),
        &ready.next.reason,
    );
    agent_push_suggestion(
        &mut suggestions,
        format!("crabdb agent finish {lane} --dry-run"),
        "preview apply and cleanup once review and validation are done",
    );
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

fn agent_file_change_cards(
    lane: &str,
    paths: &[FileDiffSummary],
    groups: &[AgentChangeGroup],
) -> Vec<AgentChangeCard> {
    let mut ranked = Vec::new();
    let mut changed_paths = paths.to_vec();
    changed_paths.sort_by(|left, right| left.path.cmp(&right.path));

    for change in changed_paths {
        let touched_groups = groups
            .iter()
            .filter(|group| {
                group
                    .changed_paths
                    .iter()
                    .any(|file| agent_file_matches_path(file, &change.path))
            })
            .collect::<Vec<_>>();
        let touched_by = touched_groups
            .iter()
            .map(|group| agent_file_touch_for_path(lane, group, &change.path))
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

        let (score, mut reasons) = agent_review_priority_score(&change, touched_by.len());
        if reasons.is_empty() {
            reasons.push("changed by the agent task".to_string());
        }
        let risk = agent_change_card_risk(score);
        let churn = change.additions + change.deletions;
        let path = change.path.clone();
        let file_arg = agent_shell_arg(&path);
        let title = path.clone();
        let summary = format!(
            "`{}`: {:?} (+{} -{}), touched by {} turn(s) or operation(s).",
            path,
            change.kind,
            change.additions,
            change.deletions,
            touched_by.len()
        );
        let diff_command = touched_by
            .first()
            .map(|touch| agent_file_diff_command(lane, touch, &path))
            .unwrap_or_else(|| format!("crabdb agent diff {lane} --file {file_arg} --patch"));

        ranked.push((
            agent_change_card_risk_rank(&risk),
            score,
            churn,
            AgentChangeCard {
                rank: 0,
                key: path.clone(),
                title,
                summary,
                risk,
                reasons,
                changed_paths: vec![change],
                touched_by,
                operations,
                tool_summaries,
                review_command: String::new(),
                focus_command: Some(format!("crabdb agent focus {lane} --file {file_arg}")),
                why_command: Some(format!("crabdb agent why {lane} {file_arg}")),
                diff_command: Some(diff_command),
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
            card.review_command = format!(
                "crabdb agent file {lane} {}",
                agent_shell_arg(card.key.as_str())
            );
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
    if grouping == "file" {
        return format!(
            "{} changed file(s) are shown as {} file review card(s), with {} turn/operation group(s) available for provenance.",
            view.task.changed_paths.len(),
            cards.len(),
            groups.len()
        );
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

fn agent_mark_file_reviewed_summary(
    task: &AgentTaskReport,
    path: &str,
    previous: Option<&AgentFileReviewMarker>,
    marker: &AgentFileReviewMarker,
) -> String {
    match previous {
        Some(previous) if previous.checkpoint == marker.checkpoint => format!(
            "`{path}` in {} was already marked reviewed at checkpoint `{}`; refreshed the file marker.",
            agent_task_label(task),
            marker.checkpoint.0
        ),
        Some(previous) => format!(
            "`{path}` in {} marked reviewed at checkpoint `{}`; previous file marker was `{}`.",
            agent_task_label(task),
            marker.checkpoint.0,
            previous.checkpoint.0
        ),
        None => format!(
            "`{path}` in {} marked reviewed at checkpoint `{}`.",
            agent_task_label(task),
            marker.checkpoint.0
        ),
    }
}

fn agent_mark_file_reviewed_suggestions(lane: &str, path: &str) -> Vec<StatusSuggestion> {
    vec![
        StatusSuggestion {
            command: format!("crabdb agent review-map {lane}"),
            reason: "continue the file-by-file review checklist".to_string(),
        },
        StatusSuggestion {
            command: format!("crabdb agent file {lane} {}", agent_shell_arg(path)),
            reason: "reopen the reviewed file context if needed".to_string(),
        },
        StatusSuggestion {
            command: format!("crabdb agent confidence {lane}"),
            reason: "finish with one go/no-go verdict after review and validation".to_string(),
        },
    ]
}

fn agent_archive_summary(
    task: &AgentTaskReport,
    archived: bool,
    previous_archived: bool,
) -> String {
    let label = agent_task_label(task);
    match (archived, previous_archived) {
        (true, true) => format!("{label} was already archived; it remains hidden from the default agent inbox."),
        (true, false) => format!("{label} archived; it is hidden from the default agent inbox and can be restored with `agent unarchive`."),
        (false, true) => format!("{label} restored; it will appear in the default agent inbox again."),
        (false, false) => format!("{label} was not archived; it remains visible in the default agent inbox."),
    }
}

fn agent_archive_suggestions(task: &AgentTaskReport, archived: bool) -> Vec<StatusSuggestion> {
    let lane = &task.lane;
    let mut suggestions = Vec::new();
    if archived {
        agent_push_suggestion(
            &mut suggestions,
            "crabdb agent inbox".to_string(),
            "return to the active task inbox without archived tasks",
        );
        agent_push_suggestion(
            &mut suggestions,
            "crabdb agent inbox --all".to_string(),
            "show active and archived tasks together",
        );
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent unarchive {lane}"),
            "restore this task to the default inbox",
        );
    } else {
        agent_push_suggestion(
            &mut suggestions,
            format!("crabdb agent view {lane}"),
            "inspect the restored task transcript, tools, and checkpoint",
        );
        agent_push_suggestion(
            &mut suggestions,
            "crabdb agent inbox".to_string(),
            "return to the active task inbox",
        );
    }
    suggestions
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

fn agent_handoff_summary(report: &AgentReviewBundleReport) -> String {
    let title = agent_task_label(&report.task);
    format!(
        "{title}: handoff packet with {} changed file(s), {} turn(s), {:?} risk, and readiness `{}`.",
        report.task.changed_paths.len(),
        report.task.turns,
        report.risk.level,
        report.readiness_status
    )
}

fn agent_handoff_markdown(
    report: &AgentReviewBundleReport,
    validation: &[LaneTestSummary],
) -> String {
    let mut out = String::new();
    let title = agent_task_label(&report.task);
    out.push_str(&format!("# Agent Task Handoff: {title}\n\n"));
    out.push_str("Use this packet to continue, review, validate, or apply the recorded agent task without reading CrabDB internals.\n\n");

    out.push_str("## Current State\n\n");
    out.push_str(&format!("- Task: `{}`\n", report.task.name));
    out.push_str(&format!("- Lane: `{}`\n", report.task.lane));
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
        "- Risk: {:?} ({}/100)\n",
        report.risk.level, report.risk.score
    ));
    out.push_str(&format!(
        "- Changed files: {}  Turns: {}  Tool events: {}\n",
        report.task.changed_paths.len(),
        report.task.turns,
        report.task.tool_events
    ));
    if let Some(checkpoint) = &report.task.latest_checkpoint {
        out.push_str(&format!("- Latest checkpoint: `{}`\n", checkpoint.0));
    }
    if let Some(workdir) = &report.task.workdir {
        out.push_str(&format!("- Workdir: `{workdir}`\n"));
    }
    out.push('\n');
    out.push_str(&format!("{}\n\n", report.story.summary));

    out.push_str("## Receiver Next Step\n\n");
    out.push_str(&format!("```sh\n{}\n```\n\n", report.next.command));
    out.push_str(&format!("{}\n\n", report.next.reason));

    out.push_str("## Review Commands\n\n");
    out.push_str(&format!("```sh\ncrabdb agent focus {}\n", report.task.lane));
    out.push_str(&format!("crabdb agent changes {}\n", report.task.lane));
    out.push_str(&format!("crabdb agent review-plan {}\n", report.task.lane));
    out.push_str(&format!("crabdb agent ready {}\n", report.task.lane));
    out.push_str(&format!(
        "crabdb agent land {} --dry-run\n",
        report.task.lane
    ));
    out.push_str("```\n\n");

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
        out.push_str("## Turn Summary\n\n");
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

    out.push_str("## Related Packets\n\n");
    out.push_str(&format!(
        "```sh\ncrabdb agent receipt {}\ncrabdb agent report {} --markdown\ncrabdb agent pr {}\n```\n",
        report.task.lane, report.task.lane, report.task.lane
    ));
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
            command: "crabdb agent setup".to_string(),
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
        AgentTaskStatus::Applied => vec![
            StatusSuggestion {
                command: format!("crabdb agent finish {lane}"),
                reason: "hide the applied task from the default inbox when you are done"
                    .to_string(),
            },
            StatusSuggestion {
                command: format!("crabdb agent receipt {lane}"),
                reason: "print the applied task receipt if you need to share it".to_string(),
            },
        ],
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
                command: "crabdb agent setup".to_string(),
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
            "finish_applied",
            "The agent task has already been applied.",
            StatusSuggestion {
                command: format!("crabdb agent finish {lane}"),
                reason: "hide the applied task from the default inbox when you are done"
                    .to_string(),
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
            format!("crabdb agent action {lane}"),
            "show the available action palette for this task",
        );
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
