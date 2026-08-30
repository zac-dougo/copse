#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use uuid::Uuid;

use crate::discovery::Worktree;
use crate::herdr::{Agent, AgentStatus, Snapshot};
use crate::tracker::Issue;

#[derive(Debug, Clone)]
pub struct AgentNode {
    pub agent: Agent,
    pub badge: String,
    pub color: Color,
}

#[derive(Debug, Clone)]
pub struct IssueNode {
    pub issue: Issue,
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

pub fn build_forest(
    worktrees: Vec<Worktree>,
    issues: Vec<Issue>,
    links: Vec<crate::tracker::Link>,
    snapshot: &Snapshot,
) -> Forest {
    let mut wt_by_path: HashMap<PathBuf, Worktree> = HashMap::new();
    for wt in &worktrees {
        wt_by_path.insert(wt.path.clone(), wt.clone());
    }

    let mut issue_by_id: HashMap<Uuid, Issue> = HashMap::new();
    for issue in &issues {
        issue_by_id.insert(issue.id, issue.clone());
    }

    let mut wt_to_issue: HashMap<PathBuf, Uuid> = HashMap::new();
    let mut issue_to_wt: HashMap<Uuid, PathBuf> = HashMap::new();
    for link in &links {
        let wt_path = PathBuf::from(&link.worktree);
        wt_to_issue.insert(wt_path.clone(), link.issue);
        issue_to_wt.insert(link.issue, wt_path);
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

        let issue_node = if let Some(issue_id) = wt_to_issue.get(&wt.path) {
            if let Some(issue) = issue_by_id.get(issue_id) {
                linked_issue_ids.insert(*issue_id);
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

    let mut unlinked_issues = Vec::new();
    for issue in issues {
        if !linked_issue_ids.contains(&issue.id) {
            unlinked_issues.push(IssueNode {
                issue,
                agents: Vec::new(),
            });
        }
    }

    Forest {
        branches,
        unlinked_issues,
    }
}

pub struct ForestWidget<'a> {
    pub forest: &'a Forest,
    pub selected: Option<usize>,
}

impl<'a> Widget for ForestWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let mut y = area.y;
        let max_y = area.y + area.height;
        let max_x = area.x + area.width;

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
            let branch_line = Line::from(vec![
                Span::styled(joint.to_string(), Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(branch.branch_name.clone(), branch_style),
            ]);
            buf.set_line(area.x + 2, y, &branch_line, area.width.saturating_sub(2));
            y += 1;
            if y >= max_y {
                break;
            }

            // Issue line
            let issue_line = if let Some(issue_node) = &branch.issue {
                Line::from(vec![
                    Span::raw("      "),
                    Span::styled("issue  ", Style::default().fg(Color::DarkGray)),
                    Span::raw(issue_node.issue.title.clone()),
                ])
            } else {
                Line::from(vec![
                    Span::raw("      "),
                    Span::styled("issue  ", Style::default().fg(Color::DarkGray)),
                    Span::styled("No linked issue", Style::default().fg(Color::DarkGray)),
                ])
            };
            buf.set_line(area.x, y, &issue_line, area.width);
            y += 1;
            if y >= max_y {
                break;
            }

            // Agent line(s) — for prototype, one line per worktree; if multiple agents, show each
            if let Some(issue_node) = &branch.issue {
                if issue_node.agents.is_empty() {
                    // No agent linked to this issue's worktree
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
                        let line = Line::from(vec![
                            Span::raw("      "),
                            Span::styled("agent  ", Style::default().fg(Color::DarkGray)),
                            Span::raw(format!("{}  ", agent_node.agent.agent)),
                            Span::styled(
                                agent_node.badge.clone(),
                                Style::default().fg(agent_node.color),
                            ),
                        ]);
                        buf.set_line(area.x, y, &line, area.width);
                        y += 1;
                    }
                }
            } else {
                // No issue, show no agent as well
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
                let line = Line::from(vec![
                    Span::raw("      "),
                    Span::styled("issue  ", Style::default().fg(Color::DarkGray)),
                    Span::raw(issue_node.issue.title.clone()),
                ]);
                buf.set_line(area.x, y, &line, area.width);
                y += 1;
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
    use crate::herdr::{AgentStatus, Snapshot};
    use crate::tracker::{Issue, Link, Status};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use std::collections::HashMap;
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

    fn test_issue(id: &str, title: &str, status: Status) -> Issue {
        Issue {
            id: Uuid::parse_str(id).unwrap(),
            title: title.to_string(),
            status,
            body: format!("Body for {title}"),
            extra: HashMap::new(),
        }
    }

    fn test_link(id: &str, issue: &str, worktree: &str) -> Link {
        Link {
            id: Uuid::parse_str(id).unwrap(),
            issue: Uuid::parse_str(issue).unwrap(),
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
        let issue = test_issue(
            "11111111-1111-4111-8111-111111111111",
            "Test Issue",
            Status::Open,
        );
        let link = test_link(
            "22222222-2222-4222-8222-222222222222",
            "11111111-1111-4111-8111-111111111111",
            "/tmp/repo",
        );
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
    fn unlinked_issue_shown_separately() {
        let wt = test_worktree("/tmp/repo", "main");
        let issue1 = test_issue(
            "11111111-1111-4111-8111-111111111111",
            "Linked",
            Status::Open,
        );
        let issue2 = test_issue(
            "33333333-3333-4333-8333-333333333333",
            "Unlinked",
            Status::Open,
        );
        let link = test_link(
            "22222222-2222-4222-8222-222222222222",
            "11111111-1111-4111-8111-111111111111",
            "/tmp/repo",
        );
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
    fn agent_mapped_to_linked_issue() {
        let wt = test_worktree("/tmp/repo", "main");
        let issue = test_issue(
            "11111111-1111-4111-8111-111111111111",
            "Issue",
            Status::Open,
        );
        let link = test_link(
            "22222222-2222-4222-8222-222222222222",
            "11111111-1111-4111-8111-111111111111",
            "/tmp/repo",
        );
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
        let issue = test_issue(
            "11111111-1111-4111-8111-111111111111",
            "My Issue",
            Status::Open,
        );
        let link = test_link(
            "22222222-2222-4222-8222-222222222222",
            "11111111-1111-4111-8111-111111111111",
            "/tmp/repo",
        );
        let mut snap = Snapshot::default();
        snap.agents = vec![test_agent("w5:p1", AgentStatus::Working, "/tmp/repo")];
        let forest = build_forest(vec![wt], vec![issue], vec![link], &snap);

        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        ForestWidget {
            forest: &forest,
            selected: None,
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
}
