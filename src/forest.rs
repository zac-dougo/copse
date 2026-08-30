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
use crate::tracker::{Issue, Status as IssueStatus};

#[derive(Debug, Clone)]
pub struct AgentNode {
    pub agent: Agent,
    pub status_symbol: String,
    pub status_color: Color,
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

pub fn status_style_and_symbol(status: AgentStatus) -> (Color, &'static str) {
    match status {
        AgentStatus::Working => (Color::Blue, "●"),
        AgentStatus::Idle | AgentStatus::Done => (Color::Green, "●"),
        AgentStatus::Blocked => (Color::Red, "●"),
        AgentStatus::Unknown => (Color::Gray, "○"),
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

    // Map worktree path -> issue id via Links (1:1, last wins on duplicate)
    let mut wt_to_issue: HashMap<PathBuf, Uuid> = HashMap::new();
    let mut issue_to_wt: HashMap<Uuid, PathBuf> = HashMap::new();
    for link in &links {
        let wt_path = PathBuf::from(&link.worktree);
        wt_to_issue.insert(wt_path.clone(), link.issue);
        issue_to_wt.insert(link.issue, wt_path);
    }

    // Map agents to worktrees by prefix matching
    let wt_paths: Vec<PathBuf> = worktrees.iter().map(|w| w.path.clone()).collect();
    let mut agents_by_wt: HashMap<PathBuf, Vec<Agent>> = HashMap::new();
    for agent in &snapshot.agents {
        let cwd_str = agent.foreground_cwd.as_ref().or(agent.cwd.as_ref());
        if let Some(cwd_str) = cwd_str {
            let cwd = PathBuf::from(cwd_str);
            // Find longest matching worktree
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

    // Build branches sorted: main first, then alphabetical by branch name or path
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
                        let (color, symbol) = status_style_and_symbol(agent.agent_status);
                        AgentNode {
                            agent,
                            status_color: color,
                            status_symbol: symbol.to_string(),
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
            // No linked issue, but still show agents under issue-less branch?
            // For forest spec, agents are children of issues, so we only show agents if there's an issue.
            // However, if there are agents but no issue, we could create a synthetic issue node?
            // For v1, we keep agents only if there's a linked issue; otherwise agents are not shown in tree.
            // This matches branch → issue → agent strictly.
            None
        };

        branches.push(BranchNode {
            worktree: wt,
            branch_name,
            is_main,
            issue: issue_node,
        });
    }

    // Unlinked issues: those not in any link
    let mut unlinked_issues = Vec::new();
    for issue in issues {
        if !linked_issue_ids.contains(&issue.id) {
            unlinked_issues.push(IssueNode {
                issue,
                agents: Vec::new(),
            });
        }
    }

    // If there are agents that mapped to a worktree but that worktree had no linked issue,
    // those agents are currently not shown. For v1, we keep that behavior (strict tree).
    // Future could show them under a placeholder.

    Forest {
        branches,
        unlinked_issues,
    }
}

// Rendering

pub struct ForestWidget<'a> {
    pub forest: &'a Forest,
    pub selected: Option<usize>, // flat index for highlight, not used yet
}

impl<'a> Widget for ForestWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut y = area.y;
        let max_y = area.y + area.height;

        for branch in &self.forest.branches {
            if y >= max_y {
                break;
            }
            // Branch line: branch name, bold if main
            let branch_style = if branch.is_main {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let branch_line = Line::from(vec![
                Span::styled("▶ ", Style::default().fg(Color::DarkGray)),
                Span::styled(branch.branch_name.clone(), branch_style),
                Span::styled(
                    format!("  {}", branch.worktree.path.display()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            buf.set_line(area.x, y, &branch_line, area.width);
            y += 1;
            if y >= max_y {
                break;
            }

            if let Some(issue) = &branch.issue {
                // Issue line: indent 2
                let issue_status_str = match issue.issue.status {
                    IssueStatus::Open => "open",
                    IssueStatus::Closed => "closed",
                    IssueStatus::Archived => "archived",
                };
                let issue_line = Line::from(vec![
                    Span::raw("  "),
                    Span::styled("○ ", Style::default().fg(Color::DarkGray)),
                    Span::raw(issue.issue.title.clone()),
                    Span::styled(
                        format!(" [{}]", issue_status_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                buf.set_line(area.x, y, &issue_line, area.width);
                y += 1;
                if y >= max_y {
                    break;
                }

                for agent_node in &issue.agents {
                    if y >= max_y {
                        break;
                    }
                    let (color, symbol) = status_style_and_symbol(agent_node.agent.agent_status);
                    let status_str = agent_node.agent.agent_status.to_string();
                    let agent_line = Line::from(vec![
                        Span::raw("    "),
                        Span::styled(format!("{} ", symbol), Style::default().fg(color)),
                        Span::raw(agent_node.agent.agent.clone()),
                        Span::styled(format!(" [{}]", status_str), Style::default().fg(color)),
                        Span::styled(
                            format!(" {}", agent_node.agent.pane_id),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]);
                    buf.set_line(area.x, y, &agent_line, area.width);
                    y += 1;
                }
            }
        }

        // Unlinked issues section
        if !self.forest.unlinked_issues.is_empty() && y < max_y {
            let header = Line::from(vec![Span::styled(
                "─ unlinked ─",
                Style::default().fg(Color::DarkGray),
            )]);
            buf.set_line(area.x, y, &header, area.width);
            y += 1;
            for issue_node in &self.forest.unlinked_issues {
                if y >= max_y {
                    break;
                }
                let issue_status_str = match issue_node.issue.status {
                    IssueStatus::Open => "open",
                    IssueStatus::Closed => "closed",
                    IssueStatus::Archived => "archived",
                };
                let line = Line::from(vec![
                    Span::raw("  "),
                    Span::styled("○ ", Style::default().fg(Color::DarkGray)),
                    Span::raw(issue_node.issue.title.clone()),
                    Span::styled(
                        format!(" [{}]", issue_status_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                buf.set_line(area.x, y, &line, area.width);
                y += 1;
            }
        }
    }
}

// Helper to render issue body markdown as plain text for detail pane.
// For v1, we just strip markdown to text via pulldown-cmark.
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
        assert_eq!(agents[0].status_color, Color::Blue);
    }

    #[test]
    fn agent_status_colors() {
        let (c, s) = status_style_and_symbol(AgentStatus::Working);
        assert_eq!(c, Color::Blue);
        assert_eq!(s, "●");
        let (c, _) = status_style_and_symbol(AgentStatus::Idle);
        assert_eq!(c, Color::Green);
        let (c, _) = status_style_and_symbol(AgentStatus::Done);
        assert_eq!(c, Color::Green);
        let (c, _) = status_style_and_symbol(AgentStatus::Blocked);
        assert_eq!(c, Color::Red);
        let (c, sym) = status_style_and_symbol(AgentStatus::Unknown);
        assert_eq!(c, Color::Gray);
        assert_eq!(sym, "○");
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

        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        ForestWidget {
            forest: &forest,
            selected: None,
        }
        .render(area, &mut buf);

        // Convert buffer to string for inspection
        let content: String = (0..area.height)
            .map(|y| {
                let line: String = (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect();
                line
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(content.contains("main"), "branch not rendered: {content}");
        assert!(content.contains("My Issue"), "issue not rendered");
        assert!(content.contains("w5:p1"), "agent pane not rendered");
        assert!(content.contains("working"), "status not rendered");
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
