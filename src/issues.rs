#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::forest::{AgentNode, status_badge};
use crate::github::{FrontierState, GitHubIssue, classify_issue};
use crate::herdr::{Agent, Snapshot};
use crate::tracker::Link;

#[derive(Debug, Clone)]
pub struct IssueRow {
    pub issue: GitHubIssue,
    pub state: FrontierState,
    pub linked: bool,
    pub agent: Option<AgentNode>,
}

#[derive(Debug, Clone, Default)]
pub struct IssuesData {
    pub rows: Vec<IssueRow>,
}

impl IssuesData {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn grouped(&self) -> Vec<(FrontierState, Vec<&IssueRow>)> {
        let mut map: HashMap<FrontierState, Vec<&IssueRow>> = HashMap::new();
        for row in &self.rows {
            map.entry(row.state).or_default().push(row);
        }
        let order = [
            FrontierState::Frontier,
            FrontierState::Blocked,
            FrontierState::Assigned,
            FrontierState::Done,
        ];
        order
            .into_iter()
            .map(|state| {
                let mut v = map.remove(&state).unwrap_or_default();
                v.sort_by_key(|r| r.issue.number);
                (state, v)
            })
            .collect()
    }

    pub fn ordered(&self) -> Vec<&IssueRow> {
        let mut ordered = Vec::new();
        for state in [
            FrontierState::Frontier,
            FrontierState::Blocked,
            FrontierState::Assigned,
            FrontierState::Done,
        ] {
            let mut v: Vec<&IssueRow> = self.rows.iter().filter(|r| r.state == state).collect();
            v.sort_by_key(|r| r.issue.number);
            ordered.extend(v);
        }
        ordered
    }
}

pub fn build_issues_data(
    mut issues: Vec<GitHubIssue>,
    links: Vec<Link>,
    snapshot: &Snapshot,
) -> IssuesData {
    // GitHub numbers are the stable identity; sort for deterministic rows.
    issues.sort_by_key(|issue| issue.number);

    let linked_numbers: HashSet<u64> = links.iter().map(|link| link.issue).collect();

    let mut agents_by_wt: HashMap<String, Vec<Agent>> = HashMap::new();
    for agent in &snapshot.agents {
        if let Some(cwd) = agent.foreground_cwd.as_ref().or(agent.cwd.as_ref()) {
            for link in &links {
                if cwd.starts_with(&link.worktree) {
                    agents_by_wt
                        .entry(link.worktree.clone())
                        .or_default()
                        .push(agent.clone());
                }
            }
        }
    }

    let mut rows = Vec::new();
    for issue in &issues {
        let is_linked = linked_numbers.contains(&issue.number);
        let state = classify_issue(issue);

        let agent = if is_linked {
            links
                .iter()
                .find(|link| link.issue == issue.number)
                .and_then(|link| agents_by_wt.get(&link.worktree))
                .and_then(|agents| agents.first())
                .map(|agent| {
                    let (color, badge) = status_badge(agent.agent_status);
                    AgentNode {
                        agent: agent.clone(),
                        badge: badge.to_string(),
                        color,
                    }
                })
        } else {
            None
        };

        rows.push(IssueRow {
            issue: issue.clone(),
            state,
            linked: is_linked,
            agent,
        });
    }

    rows.sort_by_key(|r| r.issue.number);

    IssuesData { rows }
}

pub struct IssuesWidget<'a> {
    pub data: &'a IssuesData,
    pub selected: Option<usize>,
}

impl<'a> Widget for IssuesWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let max_y = area.y + area.height;
        let footer_y = max_y - 1;
        let mut y = area.y;

        buf.set_line(
            area.x + 2,
            y,
            &Line::from(Span::styled(
                "ISSUES",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            area.width.saturating_sub(2),
        );
        y += 1;

        if y >= footer_y {
            return;
        }

        let grouped = self.data.grouped();
        let ordered = self.data.ordered();
        let _selected_row = self.selected.and_then(|idx| ordered.get(idx));

        for (state, rows) in grouped {
            if y >= footer_y {
                break;
            }
            let (label_color, label) = section_label(state);
            let header = format!("─ {} ({})", label, rows.len());
            buf.set_line(
                area.x + 2,
                y,
                &Line::from(Span::styled(header, Style::default().fg(label_color))),
                area.width.saturating_sub(2),
            );
            y += 1;

            if rows.is_empty() {
                if y < footer_y {
                    buf.set_line(
                        area.x + 2,
                        y,
                        &Line::from(Span::styled(
                            "  (none)",
                            Style::default().fg(Color::DarkGray),
                        )),
                        area.width.saturating_sub(2),
                    );
                    y += 1;
                }
                continue;
            }

            for row in rows {
                if y >= footer_y {
                    break;
                }
                // Find flat index for this row
                let flat_idx = ordered
                    .iter()
                    .position(|r| r.issue.number == row.issue.number)
                    .unwrap();
                let is_selected = self.selected == Some(flat_idx);
                write_row(area, y, row, is_selected, buf);
                y += 1;
            }
        }

        if footer_y > area.y {
            buf.set_line(
                area.x + 2,
                footer_y,
                &Line::from(Span::styled(
                    "Read this as: every GitHub issue, grouped by state.",
                    Style::default().fg(Color::DarkGray),
                )),
                area.width.saturating_sub(2),
            );
        }
    }
}

fn write_row(area: Rect, y: u16, row: &IssueRow, selected: bool, buf: &mut Buffer) {
    let color = row_color(row.state);
    let base_style = Style::default().fg(color);
    let style = if selected {
        base_style
            .bg(Color::White)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        base_style
    };

    let labels = row
        .issue
        .labels
        .iter()
        .filter(|label| *label != "wayfinder:map")
        .collect::<Vec<_>>();

    let mut spans = vec![
        Span::styled(format!("  {} ", row_symbol(row.state)), style),
        Span::styled(format!("#{} ", row.issue.number), style),
        Span::styled(row.issue.title.clone(), style),
    ];
    if !labels.is_empty() {
        spans.push(Span::styled(
            format!(
                "  [{}]",
                labels
                    .iter()
                    .map(|l| l.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            if selected {
                style.fg(Color::Black)
            } else {
                style.fg(Color::DarkGray)
            },
        ));
    }
    if !row.issue.open_blockers.is_empty() {
        let blocker_text = format!(
            "  blocked by {}",
            row.issue
                .open_blockers
                .iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        spans.push(Span::styled(
            blocker_text,
            if selected {
                style.fg(Color::Black)
            } else {
                style.fg(Color::Red)
            },
        ));
    }
    if let Some(agent) = &row.agent {
        spans.push(Span::styled(
            format!("  {} {}", agent.agent.agent, agent.badge),
            if selected {
                style.fg(Color::Black)
            } else {
                Style::default().fg(agent.color)
            },
        ));
    }

    // For selected, set gutter and bg
    if selected {
        buf[(area.x, y)].set_symbol("▸");
        buf[(area.x, y)].set_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
        // Render line with offset to leave gutter
        let line = Line::from(spans);
        buf.set_line(area.x + 2, y, &line, area.width.saturating_sub(2));
        for x in area.x..area.x + area.width {
            let s = buf[(x, y)].style().bg(Color::White).fg(Color::Black);
            // Keep the gutter's style
            if x == area.x {
                buf[(x, y)].set_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::White)
                        .add_modifier(Modifier::BOLD),
                );
            } else {
                buf[(x, y)].set_style(s);
            }
        }
    } else {
        let line = Line::from(spans);
        buf.set_line(area.x + 2, y, &line, area.width.saturating_sub(2));
    }
}

fn section_label(state: FrontierState) -> (Color, &'static str) {
    match state {
        FrontierState::Frontier => (Color::Green, "Frontier"),
        FrontierState::Blocked => (Color::Red, "Blocked"),
        FrontierState::Assigned => (Color::Cyan, "Assigned"),
        FrontierState::Done => (Color::DarkGray, "Done"),
    }
}

fn row_color(state: FrontierState) -> Color {
    match state {
        FrontierState::Frontier => Color::Green,
        FrontierState::Blocked => Color::Red,
        FrontierState::Assigned => Color::Cyan,
        FrontierState::Done => Color::Gray,
    }
}

fn row_symbol(state: FrontierState) -> &'static str {
    match state {
        FrontierState::Frontier => "→",
        FrontierState::Blocked => "!",
        FrontierState::Assigned => "○",
        FrontierState::Done => "✓",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::IssueState;
    use crate::herdr::{AgentStatus, Snapshot};
    use crate::tracker::Link;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use uuid::Uuid;

    fn test_issue(number: u64, title: &str, state: IssueState) -> GitHubIssue {
        GitHubIssue {
            number,
            title: title.to_string(),
            state,
            body: String::new(),
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

    fn test_agent(cwd: &str) -> Agent {
        Agent {
            agent: "pi".to_string(),
            agent_status: AgentStatus::Working,
            cwd: Some(cwd.to_string()),
            foreground_cwd: Some(cwd.to_string()),
            pane_id: "w1:p1".to_string(),
            workspace_id: "w1".to_string(),
            tab_id: "w1:t1".to_string(),
            terminal_id: None,
            focused: None,
        }
    }

    #[test]
    fn groups_every_issue_by_state() {
        let mut blocked = test_issue(2, "Blocked", IssueState::Open);
        blocked.open_blockers = vec![5];
        let mut assigned = test_issue(3, "Assigned", IssueState::Open);
        assigned.assignees = vec!["zac".to_string()];

        let data = build_issues_data(
            vec![
                test_issue(1, "Frontier", IssueState::Open),
                blocked,
                assigned,
                test_issue(4, "Done", IssueState::Closed),
                test_issue(5, "Blocker", IssueState::Open),
            ],
            vec![test_link(3, "/tmp/repo")],
            &Snapshot::default(),
        );

        let grouped = data.grouped();
        let titles = |state| {
            grouped
                .iter()
                .find(|(s, _)| *s == state)
                .unwrap()
                .1
                .iter()
                .map(|r| r.issue.title.as_str())
                .collect::<Vec<_>>()
        };
        // Blocker itself is also frontier (open, unblocked, unassigned).
        assert_eq!(titles(FrontierState::Frontier), vec!["Frontier", "Blocker"]);
        assert_eq!(titles(FrontierState::Blocked), vec!["Blocked"]);
        assert_eq!(titles(FrontierState::Assigned), vec!["Assigned"]);
        assert_eq!(titles(FrontierState::Done), vec!["Done"]);
    }

    #[test]
    fn linked_issue_carries_agent() {
        let mut snapshot = Snapshot::default();
        snapshot.agents = vec![test_agent("/tmp/repo")];
        let data = build_issues_data(
            vec![test_issue(3, "Assigned", IssueState::Open)],
            vec![test_link(3, "/tmp/repo")],
            &snapshot,
        );
        assert_eq!(data.rows.len(), 1);
        assert!(data.rows[0].linked);
        assert_eq!(data.rows[0].agent.as_ref().unwrap().agent.pane_id, "w1:p1");
    }

    #[test]
    fn renders_grouped_sections() {
        let issues = vec![
            test_issue(1, "Frontier Issue", IssueState::Open),
            test_issue(2, "Done Issue", IssueState::Closed),
        ];
        let data = build_issues_data(issues, vec![], &Snapshot::default());
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        IssuesWidget {
            data: &data,
            selected: None,
        }
        .render(area, &mut buf);
        let content: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(content.contains("ISSUES"));
        assert!(content.contains("─ Frontier (1)"));
        assert!(content.contains("─ Done (1)"));
        assert!(content.contains("Frontier Issue"));
        assert!(content.contains("Done Issue"));
    }

    #[test]
    fn selected_row_is_highlighted() {
        let issues = vec![test_issue(1, "Issue", IssueState::Open)];
        let data = build_issues_data(issues, vec![], &Snapshot::default());
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        IssuesWidget {
            data: &data,
            selected: Some(0),
        }
        .render(area, &mut buf);
        let y = (0..area.height)
            .find(|y| {
                (0..area.width)
                    .map(|x| buf[(x, *y)].symbol().to_string())
                    .collect::<String>()
                    .contains("Issue")
            })
            .unwrap();
        assert_eq!(buf[(0, y)].bg, Color::White);
        assert_eq!(buf[(0, y)].symbol(), "▸");
    }
}
