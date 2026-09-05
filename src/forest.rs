#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::discovery::Worktree;
use crate::github::{GitHubIssue, IssueState};
use crate::herdr::{Agent, AgentStatus, Snapshot};
use crate::tracker::Link;

#[derive(Debug, Clone)]
pub struct AgentNode {
    pub agent: Agent,
    pub badge: String,
    pub color: Color,
}

#[derive(Debug, Clone)]
pub struct IssueNode {
    pub issue: GitHubIssue,
    pub agents: Vec<AgentNode>,
}

#[derive(Debug, Clone)]
pub struct BranchNode {
    pub worktree: Worktree,
    pub branch_name: String,
    pub is_main: bool,
    pub issue: Option<IssueNode>,
}

#[derive(Debug, Clone, Default)]
pub struct Forest {
    pub branches: Vec<BranchNode>,
    pub unlinked_issues: Vec<IssueNode>,
}

impl Forest {
    pub fn is_empty(&self) -> bool {
        self.branches.is_empty() && self.unlinked_issues.is_empty()
    }
}

pub fn status_badge(status: AgentStatus) -> (Color, &'static str) {
    match status {
        AgentStatus::Working => (Color::Blue, "● working"),
        AgentStatus::Done => (Color::Green, "✓ done"),
        AgentStatus::Blocked => (Color::Red, "! blocked"),
        AgentStatus::Idle => (Color::Green, "○ idle"),
        AgentStatus::Unknown => (Color::Gray, "· unknown"),
    }
}

pub fn branch_is_main(branch: &Option<String>) -> bool {
    match branch {
        Some(b) => {
            let short = b.strip_prefix("refs/heads/").unwrap_or(b);
            short == "main" || short == "master"
        }
        None => false,
    }
}

fn has_map_label(issue: &GitHubIssue) -> bool {
    issue.labels.iter().any(|l| l == "wayfinder:map")
}

pub fn build_forest(
    worktrees: Vec<Worktree>,
    issues: Vec<GitHubIssue>,
    links: Vec<Link>,
    snapshot: &Snapshot,
) -> Forest {
    let mut wt_by_path: HashMap<PathBuf, Worktree> = HashMap::new();
    for wt in &worktrees {
        wt_by_path.insert(wt.path.clone(), wt.clone());
    }

    let mut issue_by_number: HashMap<u64, GitHubIssue> = HashMap::new();
    for issue in &issues {
        issue_by_number.insert(issue.number, issue.clone());
    }

    let mut wt_to_issue: HashMap<PathBuf, u64> = HashMap::new();
    let mut issue_to_wt: HashMap<u64, PathBuf> = HashMap::new();
    // Sort links for deterministic winner when multiple links share a worktree and
    // have the same priority (e.g. two closed or two open). Without sorting,
    // read_dir hash order would make the winner nondeterministic.
    let mut sorted_links = links.clone();
    sorted_links.sort_by_key(|l| l.issue);
    for link in &sorted_links {
        let wt_path = PathBuf::from(&link.worktree);
        let should_insert = match wt_to_issue.get(&wt_path).cloned() {
            None => true,
            Some(existing_id) => {
                let existing = issue_by_number.get(&existing_id);
                let incoming = issue_by_number.get(&link.issue);
                match (existing.map(|i| i.state), incoming.map(|i| i.state)) {
                    // Prefer Open over Closed regardless of insertion order.
                    // This fixes the main-worktree claim bug where a new open claim
                    // is hidden behind a stale closed link to the same worktree.
                    (Some(IssueState::Closed) | None, Some(IssueState::Open)) => true,
                    (Some(IssueState::Open), Some(IssueState::Closed) | None) => false,
                    _ => false,
                }
            }
        };
        if should_insert {
            // If we replaced an existing entry, also remove its issue_to_wt mapping.
            if let Some(prev) = wt_to_issue.insert(wt_path.clone(), link.issue) {
                issue_to_wt.remove(&prev);
            }
            issue_to_wt.insert(link.issue, wt_path);
        } else {
            // Even when not winning the worktree slot, track the issue->worktree link for completeness,
            // but it won't be counted as the branch's primary linked issue.
            // Don't insert into wt_to_issue; keep existing winner.
            // Still record issue_to_wt so the issue isn't considered unlinked if caller inspects,
            // but linked_issue_ids is derived from wt_to_issue winners only, so shadowed closed links become unlinked.
        }
    }

    let wt_paths: Vec<PathBuf> = worktrees.iter().map(|w| w.path.clone()).collect();
    let mut agents_by_wt: HashMap<PathBuf, Vec<Agent>> = HashMap::new();
    for agent in &snapshot.agents {
        let cwd_str = agent.foreground_cwd.as_ref().or(agent.cwd.as_ref());
        if let Some(cwd_str) = cwd_str {
            let cwd = PathBuf::from(cwd_str);
            let mut best: Option<PathBuf> = None;
            let mut best_len = 0;
            for wt_path in &wt_paths {
                if cwd.starts_with(wt_path) {
                    let len = wt_path.as_os_str().len();
                    if len > best_len {
                        best = Some(wt_path.clone());
                        best_len = len;
                    }
                }
            }
            if let Some(wt_path) = best {
                agents_by_wt.entry(wt_path).or_default().push(agent.clone());
            }
        }
    }

    let mut sorted_wts = worktrees.clone();
    sorted_wts.sort_by(|a, b| {
        let a_main = branch_is_main(&a.branch);
        let b_main = branch_is_main(&b.branch);
        match (a_main, b_main) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => {
                let a_name = a
                    .branch
                    .clone()
                    .unwrap_or_else(|| a.path.display().to_string());
                let b_name = b
                    .branch
                    .clone()
                    .unwrap_or_else(|| b.path.display().to_string());
                a_name.cmp(&b_name)
            }
        }
    });

    let mut branches = Vec::new();
    let mut linked_issue_ids = HashSet::new();

    for wt in sorted_wts {
        let branch_name = wt
            .branch
            .as_ref()
            .map(|b| b.strip_prefix("refs/heads/").unwrap_or(b).to_string())
            .unwrap_or_else(|| wt.path.display().to_string());
        let is_main = branch_is_main(&wt.branch);

        let issue_node = if let Some(issue_number) = wt_to_issue.get(&wt.path) {
            if let Some(issue) = issue_by_number.get(issue_number) {
                linked_issue_ids.insert(*issue_number);
                let agents = agents_by_wt
                    .remove(&wt.path)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|agent| {
                        let (color, badge) = status_badge(agent.agent_status);
                        AgentNode {
                            agent,
                            badge: badge.to_string(),
                            color,
                        }
                    })
                    .collect();
                Some(IssueNode {
                    issue: issue.clone(),
                    agents,
                })
            } else {
                None
            }
        } else {
            None
        };

        branches.push(BranchNode {
            worktree: wt,
            branch_name,
            is_main,
            issue: issue_node,
        });
    }

    // Unlinked section: open, unblocked GitHub issues with no worktree link.
    // Wayfinder maps are meta-issues shown in the Map view, not here.
    let mut unlinked_issues = Vec::new();
    for issue in issues {
        if linked_issue_ids.contains(&issue.number) {
            continue;
        }
        if issue.state != IssueState::Open {
            continue;
        }
        if !issue.open_blockers.is_empty() {
            continue;
        }
        if has_map_label(&issue) {
            continue;
        }
        unlinked_issues.push(IssueNode {
            issue,
            agents: Vec::new(),
        });
    }
    unlinked_issues.sort_by_key(|node| node.issue.number);

    Forest {
        branches,
        unlinked_issues,
    }
}

pub struct ForestWidget<'a> {
    pub forest: &'a Forest,
    pub selected: Option<usize>,
    pub expanded_branches: Option<&'a HashSet<PathBuf>>,
    pub expanded_issues: Option<&'a HashSet<u64>>,
}

impl<'a> Widget for ForestWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let mut y = area.y;
        let max_y = area.y + area.height;
        let max_x = area.x + area.width;
        // Track flat index for selection highlight. Flat nodes are branch, issue,
        // agent in that order, respecting expanded state. Headers and blank rows
        // are not flat nodes.
        let mut flat_idx: usize = 0;
        let is_selected = |idx: usize| self.selected == Some(idx);
        let highlight = |mut style: Style, selected: bool| {
            if selected {
                style = style
                    .bg(Color::White)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD);
            }
            style
        };

        // Header: repository
        if y < max_y {
            let line = Line::from(vec![Span::styled(
                "repository",
                Style::default().add_modifier(Modifier::BOLD),
            )]);
            buf.set_line(area.x + 2, y, &line, area.width.saturating_sub(2));
            y += 1;
        }
        // Blank line after header
        if y < max_y {
            y += 1;
        }

        for (idx, branch) in self.forest.branches.iter().enumerate() {
            if y >= max_y {
                break;
            }
            let is_last =
                idx == self.forest.branches.len() - 1 && self.forest.unlinked_issues.is_empty();
            let joint = if is_last { "└─" } else { "├─" };
            let branch_style = if branch.is_main {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            // Branch line is a flat node - check selection
            let sel_branch = is_selected(flat_idx);
            let branch_line = Line::from(vec![
                Span::styled(
                    joint.to_string(),
                    highlight(Style::default().fg(Color::DarkGray), sel_branch),
                ),
                Span::raw(" "),
                Span::styled(
                    branch.branch_name.clone(),
                    highlight(branch_style, sel_branch),
                ),
            ]);
            // Use a gutter indicator for selected branch
            let branch_x = if sel_branch { area.x } else { area.x + 2 };
            let branch_width = if sel_branch {
                area.width
            } else {
                area.width.saturating_sub(2)
            };
            if sel_branch {
                // Draw a marker in the gutter
                buf.set_line(
                    branch_x + 2,
                    y,
                    &branch_line,
                    branch_width.saturating_sub(2),
                );
                for x in area.x..area.x + area.width {
                    let s = buf[(x, y)].style().bg(Color::White).fg(Color::Black);
                    buf[(x, y)].set_style(s);
                }
                buf[(area.x, y)].set_symbol("▸");
                buf[(area.x, y)].set_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::White)
                        .add_modifier(Modifier::BOLD),
                );
            } else {
                buf.set_line(area.x + 2, y, &branch_line, area.width.saturating_sub(2));
            }
            y += 1;
            flat_idx += 1;
            if y >= max_y {
                break;
            }

            // Respect collapsed state: if branch is collapsed, skip issue/agent rendering
            let branch_expanded = self
                .expanded_branches
                .is_none_or(|set| set.contains(&branch.worktree.path));
            if !branch_expanded {
                if y < max_y {
                    y += 1;
                }
                continue;
            }

            // Issue line - only a flat node if branch has an issue
            if let Some(issue_node) = &branch.issue {
                let sel_issue = is_selected(flat_idx);
                let issue_line = Line::from(vec![
                    Span::styled(
                        "      ",
                        highlight(Style::default().fg(Color::DarkGray), sel_issue),
                    ),
                    Span::styled(
                        "issue  ",
                        highlight(Style::default().fg(Color::DarkGray), sel_issue),
                    ),
                    Span::styled(
                        issue_node.issue.title.clone(),
                        highlight(Style::default().fg(Color::White), sel_issue),
                    ),
                ]);
                if sel_issue {
                    buf.set_line(area.x, y, &issue_line, area.width);
                    for x in area.x..area.x + area.width {
                        let s = buf[(x, y)].style().bg(Color::White).fg(Color::Black);
                        buf[(x, y)].set_style(s);
                    }
                    buf[(area.x, y)].set_symbol("▸");
                    buf[(area.x, y)].set_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .bg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    );
                } else {
                    buf.set_line(area.x, y, &issue_line, area.width);
                }
                y += 1;
                flat_idx += 1;
                if y >= max_y {
                    break;
                }

                let issue_expanded = self
                    .expanded_issues
                    .is_none_or(|set| set.contains(&issue_node.issue.number));
                if !issue_expanded {
                    if y < max_y {
                        // still need to handle agent placeholder? No, collapsed issue hides agents
                    }
                } else {
                    // Agent line(s) — for prototype, one line per worktree; if multiple agents, show each
                    if issue_node.agents.is_empty() {
                        // No agent linked - this is not a flat node, just a placeholder
                        let line = Line::from(vec![
                            Span::raw("      "),
                            Span::styled("agent  ", Style::default().fg(Color::DarkGray)),
                            Span::styled("—  ", Style::default().fg(Color::DarkGray)),
                            Span::styled("· no agent", Style::default().fg(Color::DarkGray)),
                        ]);
                        buf.set_line(area.x, y, &line, area.width);
                        y += 1;
                    } else {
                        for agent_node in &issue_node.agents {
                            if y >= max_y {
                                break;
                            }
                            let sel_agent = is_selected(flat_idx);
                            let line = Line::from(vec![
                                Span::styled(
                                    "      ",
                                    highlight(Style::default().fg(Color::DarkGray), sel_agent),
                                ),
                                Span::styled(
                                    "agent  ",
                                    highlight(Style::default().fg(Color::DarkGray), sel_agent),
                                ),
                                Span::styled(
                                    format!("{}  ", agent_node.agent.agent),
                                    highlight(Style::default().fg(Color::White), sel_agent),
                                ),
                                Span::styled(
                                    agent_node.badge.clone(),
                                    highlight(Style::default().fg(agent_node.color), sel_agent),
                                ),
                            ]);
                            if sel_agent {
                                buf.set_line(area.x, y, &line, area.width);
                                for x in area.x..area.x + area.width {
                                    let s = buf[(x, y)].style().bg(Color::White).fg(Color::Black);
                                    buf[(x, y)].set_style(s);
                                }
                                buf[(area.x, y)].set_symbol("▸");
                                buf[(area.x, y)].set_style(
                                    Style::default()
                                        .fg(Color::Cyan)
                                        .bg(Color::White)
                                        .add_modifier(Modifier::BOLD),
                                );
                            } else {
                                buf.set_line(area.x, y, &line, area.width);
                            }
                            y += 1;
                            flat_idx += 1;
                        }
                    }
                }
            } else {
                // No linked issue - placeholder, not a flat node
                let line = Line::from(vec![
                    Span::raw("      "),
                    Span::styled("issue  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("No linked issue", Style::default().fg(Color::DarkGray)),
                ]);
                buf.set_line(area.x, y, &line, area.width);
                y += 1;
                if y >= max_y {
                    break;
                }
                let line = Line::from(vec![
                    Span::raw("      "),
                    Span::styled("agent  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("—  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("· no agent", Style::default().fg(Color::DarkGray)),
                ]);
                buf.set_line(area.x, y, &line, area.width);
                y += 1;
            }

            // Blank row between worktrees (spaced look)
            if y < max_y {
                y += 1;
            }
        }

        // Unlinked issues section
        if !self.forest.unlinked_issues.is_empty() && y + 1 < max_y {
            let header = Line::from(vec![Span::styled(
                "─ unlinked ─",
                Style::default().fg(Color::DarkGray),
            )]);
            buf.set_line(area.x + 2, y, &header, area.width.saturating_sub(2));
            y += 1;
            for issue_node in &self.forest.unlinked_issues {
                if y >= max_y {
                    break;
                }
                let sel_unlinked = is_selected(flat_idx);
                let line = Line::from(vec![
                    Span::styled(
                        "      ",
                        highlight(Style::default().fg(Color::DarkGray), sel_unlinked),
                    ),
                    Span::styled(
                        "issue  ",
                        highlight(Style::default().fg(Color::DarkGray), sel_unlinked),
                    ),
                    Span::styled(
                        issue_node.issue.title.clone(),
                        highlight(Style::default().fg(Color::White), sel_unlinked),
                    ),
                ]);
                if sel_unlinked {
                    buf.set_line(area.x, y, &line, area.width);
                    for x in area.x..area.x + area.width {
                        let s = buf[(x, y)].style().bg(Color::White).fg(Color::Black);
                        buf[(x, y)].set_style(s);
                    }
                    buf[(area.x, y)].set_symbol("▸");
                    buf[(area.x, y)].set_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .bg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    );
                } else {
                    buf.set_line(area.x, y, &line, area.width);
                }
                y += 1;
                flat_idx += 1;
            }
        }

        // Footer hint
        if y + 1 < max_y {
            y += 1;
            let footer = Line::from(vec![Span::styled(
                "Read this as: branch → issue → agent. Empty links are visible gaps.",
                Style::default().fg(Color::DarkGray),
            )]);
            buf.set_line(area.x + 2, y, &footer, area.width.saturating_sub(2));
        }

        // Selection highlight: dim overlay for selected row is handled by App's flat index,
        // but we keep forest widget simple. The App will draw highlight via style if needed.
        let _ = max_x;
    }
}

pub fn render_markdown_to_text(md: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let parser = Parser::new(md);
    let mut text = String::new();
    for event in parser {
        match event {
            Event::Text(t) => text.push_str(&t),
            Event::Code(t) => text.push_str(&t),
            Event::Html(_) => {}
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => text.push('\n'),
            Event::Start(Tag::Heading { .. }) => {}
            Event::End(TagEnd::Heading(_)) => text.push('\n'),
            Event::Start(Tag::List(_)) => {}
            Event::End(TagEnd::List(_)) => text.push('\n'),
            Event::Start(Tag::Item) => text.push_str("• "),
            Event::End(TagEnd::Item) => text.push('\n'),
            _ => {}
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{GitHubComment, IssueState};
    use crate::herdr::{AgentStatus, Snapshot};
    use crate::tracker::Link;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn test_worktree(path: &str, branch: &str) -> Worktree {
        Worktree {
            path: PathBuf::from(path),
            head: "abc123".to_string(),
            branch: Some(format!("refs/heads/{branch}")),
            is_bare: false,
            is_detached: false,
            is_prunable: false,
        }
    }

    fn test_issue(number: u64, title: &str, state: IssueState) -> GitHubIssue {
        GitHubIssue {
            number,
            title: title.to_string(),
            state,
            body: format!("Body for {title}"),
            comments: Vec::new(),
            labels: Vec::new(),
            assignees: Vec::new(),
            open_blockers: Vec::new(),
        }
    }

    fn test_link(issue: u64, worktree: &str) -> Link {
        Link {
            id: Uuid::new_v4(),
            issue,
            worktree: worktree.to_string(),
            body: "".to_string(),
            extra: HashMap::new(),
        }
    }

    fn test_agent(pane_id: &str, status: AgentStatus, cwd: &str) -> Agent {
        Agent {
            agent: "pi".to_string(),
            agent_status: status,
            cwd: Some(cwd.to_string()),
            foreground_cwd: Some(cwd.to_string()),
            pane_id: pane_id.to_string(),
            workspace_id: "w1".to_string(),
            tab_id: "w1:t1".to_string(),
            terminal_id: None,
            focused: None,
        }
    }

    #[test]
    fn branch_main_is_bold_and_first() {
        let wts = vec![
            test_worktree("/tmp/b", "feature"),
            test_worktree("/tmp/a", "main"),
        ];
        let forest = build_forest(wts, vec![], vec![], &Snapshot::default());
        assert_eq!(forest.branches.len(), 2);
        assert_eq!(forest.branches[0].branch_name, "main");
        assert!(forest.branches[0].is_main);
        assert!(!forest.branches[1].is_main);
    }

    #[test]
    fn links_issue_to_worktree() {
        let wt = test_worktree("/tmp/repo", "main");
        let issue = test_issue(42, "Test Issue", IssueState::Open);
        let link = test_link(42, "/tmp/repo");
        let forest = build_forest(vec![wt], vec![issue], vec![link], &Snapshot::default());
        assert_eq!(forest.branches.len(), 1);
        assert!(forest.branches[0].issue.is_some());
        assert_eq!(
            forest.branches[0].issue.as_ref().unwrap().issue.title,
            "Test Issue"
        );
        assert!(forest.unlinked_issues.is_empty());
    }

    #[test]
    fn link_to_missing_issue_leaves_branch_empty() {
        let wt = test_worktree("/tmp/repo", "main");
        let link = test_link(99, "/tmp/repo");
        let forest = build_forest(vec![wt], vec![], vec![link], &Snapshot::default());
        assert!(forest.branches[0].issue.is_none());
    }

    #[test]
    fn unlinked_issue_shown_separately() {
        let wt = test_worktree("/tmp/repo", "main");
        let issue1 = test_issue(1, "Linked", IssueState::Open);
        let issue2 = test_issue(2, "Unlinked", IssueState::Open);
        let link = test_link(1, "/tmp/repo");
        let forest = build_forest(
            vec![wt],
            vec![issue1, issue2],
            vec![link],
            &Snapshot::default(),
        );
        assert_eq!(
            forest.branches[0].issue.as_ref().unwrap().issue.title,
            "Linked"
        );
        assert_eq!(forest.unlinked_issues.len(), 1);
        assert_eq!(forest.unlinked_issues[0].issue.title, "Unlinked");
    }

    #[test]
    fn unlinked_hides_closed_blocked_and_maps() {
        let wt = test_worktree("/tmp/repo", "main");
        let mut blocked = test_issue(2, "Blocked", IssueState::Open);
        blocked.open_blockers = vec![3];
        let mut map = test_issue(4, "Map", IssueState::Open);
        map.labels = vec!["wayfinder:map".to_string()];
        let issues = vec![
            test_issue(1, "Frontier", IssueState::Open),
            blocked,
            test_issue(3, "Blocker", IssueState::Open),
            map,
            test_issue(5, "Closed", IssueState::Closed),
        ];
        let forest = build_forest(vec![wt], issues, vec![], &Snapshot::default());
        let titles: Vec<&str> = forest
            .unlinked_issues
            .iter()
            .map(|n| n.issue.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Frontier", "Blocker"]);
    }

    #[test]
    fn agent_mapped_to_linked_issue() {
        let wt = test_worktree("/tmp/repo", "main");
        let issue = test_issue(7, "Issue", IssueState::Open);
        let link = test_link(7, "/tmp/repo");
        let mut snap = Snapshot::default();
        snap.agents = vec![test_agent("w1:p1", AgentStatus::Working, "/tmp/repo")];
        let forest = build_forest(vec![wt], vec![issue], vec![link], &snap);
        let agents = &forest.branches[0].issue.as_ref().unwrap().agents;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent.pane_id, "w1:p1");
        assert_eq!(agents[0].color, Color::Blue);
    }

    #[test]
    fn agent_status_badge() {
        let (c, s) = status_badge(AgentStatus::Working);
        assert_eq!(c, Color::Blue);
        assert_eq!(s, "● working");
        let (c, s) = status_badge(AgentStatus::Idle);
        assert_eq!(c, Color::Green);
        assert_eq!(s, "○ idle");
        let (c, s) = status_badge(AgentStatus::Done);
        assert_eq!(c, Color::Green);
        assert_eq!(s, "✓ done");
        let (c, s) = status_badge(AgentStatus::Blocked);
        assert_eq!(c, Color::Red);
        assert_eq!(s, "! blocked");
        let (c, s) = status_badge(AgentStatus::Unknown);
        assert_eq!(c, Color::Gray);
        assert_eq!(s, "· unknown");
    }

    #[test]
    fn render_forest_contains_expected_text() {
        let wt = test_worktree("/tmp/repo", "main");
        let issue = test_issue(7, "My Issue", IssueState::Open);
        let link = test_link(7, "/tmp/repo");
        let mut snap = Snapshot::default();
        snap.agents = vec![test_agent("w5:p1", AgentStatus::Working, "/tmp/repo")];
        let forest = build_forest(vec![wt], vec![issue], vec![link], &snap);

        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        ForestWidget {
            forest: &forest,
            selected: None,
            expanded_branches: None,
            expanded_issues: None,
        }
        .render(area, &mut buf);

        let content: String = (0..area.height)
            .map(|y| {
                let line: String = (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect();
                line
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(content.contains("repository"), "header missing: {content}");
        assert!(content.contains("main"), "branch not rendered: {content}");
        assert!(
            content.contains("issue  My Issue"),
            "issue not rendered: {content}"
        );
        assert!(
            content.contains("agent  pi"),
            "agent not rendered: {content}"
        );
        assert!(
            content.contains("● working"),
            "badge not rendered: {content}"
        );
        assert!(
            content.contains("Read this as: branch → issue → agent"),
            "footer missing: {content}"
        );
    }

    #[test]
    fn multiple_links_to_main_prefers_open_issue() {
        // Main has a stale closed link plus a fresh open claim: the open
        // claimed issue wins the branch regardless of link order.
        let wt = test_worktree("/tmp/repo", "main");
        let closed_issue = test_issue(1, "Closed old", IssueState::Closed);
        let open_issue = test_issue(2, "Open claimed", IssueState::Open);
        let open_link = test_link(2, "/tmp/repo");
        let closed_link = test_link(1, "/tmp/repo");
        let mut snap = Snapshot::default();
        snap.agents = vec![test_agent("w1:p1", AgentStatus::Working, "/tmp/repo")];
        // Order: open first, closed last, so a naive last-wins pick is closed.
        let forest = build_forest(
            vec![wt],
            vec![closed_issue, open_issue],
            vec![open_link, closed_link],
            &snap,
        );
        let branch_issue = forest.branches[0].issue.as_ref().unwrap();
        assert_eq!(branch_issue.issue.title, "Open claimed");
        assert_eq!(branch_issue.agents.len(), 1);
        assert_eq!(branch_issue.agents[0].agent.pane_id, "w1:p1");
    }

    #[test]
    fn selected_forest_row_is_highlighted() {
        let wt = test_worktree("/tmp/repo", "main");
        let issue = test_issue(7, "My Issue", IssueState::Open);
        let link = test_link(7, "/tmp/repo");
        let mut snap = Snapshot::default();
        snap.agents = vec![test_agent("w1:p1", AgentStatus::Working, "/tmp/repo")];
        let forest = build_forest(vec![wt], vec![issue], vec![link], &snap);
        // Flat order is branch(0), issue(1), agent(2). Test each selection.
        for (selected, expected_y) in [(0usize, 2), (1, 3), (2, 4)] {
            let area = Rect::new(0, 0, 80, 20);
            let mut buf = Buffer::empty(area);
            ForestWidget {
                forest: &forest,
                selected: Some(selected),
                expanded_branches: None,
                expanded_issues: None,
            }
            .render(area, &mut buf);
            // Selected row should have White bg and gutter marker
            assert_eq!(
                buf[(0, expected_y)].bg,
                Color::White,
                "selected {selected} should highlight y {expected_y}"
            );
            assert_eq!(buf[(0, expected_y)].symbol(), "▸");
            // Other rows should not be highlighted
            for y in [2, 3, 4] {
                if y != expected_y {
                    assert_ne!(
                        buf[(0, y)].bg,
                        Color::White,
                        "unselected y {y} should not be highlighted when selected is {selected}"
                    );
                }
            }
        }
        // No selection => no highlight
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        ForestWidget {
            forest: &forest,
            selected: None,
            expanded_branches: None,
            expanded_issues: None,
        }
        .render(area, &mut buf);
        for y in [2, 3, 4] {
            assert_ne!(buf[(0, y)].bg, Color::White);
        }
    }

    #[test]
    fn markdown_to_text() {
        let md = "# Title\n\nSome **bold** text.\n\n• item";
        let text = render_markdown_to_text(md);
        assert!(text.contains("Title"));
        assert!(text.contains("Some bold text"));
    }

    #[test]
    fn forest_empty() {
        let forest = build_forest(vec![], vec![], vec![], &Snapshot::default());
        assert!(forest.is_empty());
    }

    #[test]
    fn comments_do_not_break_issue_nodes() {
        // GitHub issues carry comments; forest nodes must hold them through.
        let wt = test_worktree("/tmp/repo", "main");
        let mut issue = test_issue(7, "Discussed", IssueState::Open);
        issue.comments = vec![GitHubComment {
            author: "zac".to_string(),
            body: "looks good".to_string(),
        }];
        let link = test_link(7, "/tmp/repo");
        let forest = build_forest(vec![wt], vec![issue], vec![link], &Snapshot::default());
        assert_eq!(
            forest.branches[0]
                .issue
                .as_ref()
                .unwrap()
                .issue
                .comments
                .len(),
            1
        );
    }
}
