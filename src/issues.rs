#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use uuid::Uuid;

use crate::forest::{AgentNode, status_badge};
use crate::github::{FrontierState, GitHubIssue, IssueState, classify_issue};
use crate::herdr::{Agent, Snapshot};
use crate::tracker::{Issue, Status};

#[derive(Debug, Clone)]
pub struct IssueRow {
    pub issue: Issue,
    pub state: FrontierState,
    pub blockers: Vec<Uuid>,
    pub linked: bool,
    pub agent: Option<AgentNode>,
    pub number: u64,
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
                // Already sorted by number in build, but ensure
                v.sort_by_key(|r| r.number);
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
            v.sort_by_key(|r| r.number);
            ordered.extend(v);
        }
        ordered
    }
}

fn issue_labels(issue: &Issue) -> Vec<String> {
    issue
        .extra
        .get("labels")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(str::to_string)
        .collect()
}

fn parse_uuid_references(body: &str) -> Vec<Uuid> {
    body.lines()
        .take(12)
        .filter_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .find("blocked by:")
                .map(|index| &line[index + "blocked by:".len()..])
        })
        .flat_map(|value| value.split(|ch: char| ch == ',' || ch.is_whitespace()))
        .filter_map(|part| {
            Uuid::parse_str(part.trim_matches(|ch: char| !ch.is_ascii_hexdigit() && ch != '-')).ok()
        })
        .collect()
}

pub fn build_issues_data(
    mut issues: Vec<Issue>,
    links: Vec<crate::tracker::Link>,
    snapshot: &Snapshot,
) -> IssuesData {
    // Sort issues by id for stable number assignment, as done in github.rs
    issues.sort_by_key(|issue| issue.id);

    let numbers: HashMap<Uuid, u64> = issues
        .iter()
        .enumerate()
        .map(|(idx, issue)| (issue.id, idx as u64 + 1))
        .collect();

    let by_id: HashMap<Uuid, &Issue> = issues.iter().map(|issue| (issue.id, issue)).collect();

    let linked_ids: HashSet<Uuid> = links.iter().map(|link| link.issue).collect();

    // Map worktree path to agents, similar to forest
    let _worktrees: Vec<PathBuf> = Vec::new();
    // We don't have worktrees here, but we can map via link.worktree string to agents
    // Instead, build a map from worktree path string to agents
    let mut agents_by_wt: HashMap<String, Vec<Agent>> = HashMap::new();
    for agent in &snapshot.agents {
        if let Some(cwd) = agent.foreground_cwd.as_ref().or(agent.cwd.as_ref()) {
            // Find the link worktree that is prefix of cwd
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
    // For more accurate mapping, also consider exact worktree path
    // If no links, agents won't be mapped; that's fine for now

    let mut rows = Vec::new();
    for issue in &issues {
        let blockers = parse_uuid_references(&issue.body)
            .into_iter()
            .filter(|id| {
                by_id
                    .get(id)
                    .map(|blocker| blocker.status != Status::Closed)
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();

        let is_linked = linked_ids.contains(&issue.id);
        let number = numbers[&issue.id];

        // Build a synthetic GitHubIssue for classification, as done in github.rs
        let github_issue = GitHubIssue {
            number,
            title: issue.title.clone(),
            state: match issue.status {
                Status::Open => IssueState::Open,
                Status::Closed | Status::Archived => IssueState::Closed,
            },
            body: issue.body.clone(),
            comments: Vec::new(),
            labels: issue_labels(issue),
            assignees: if is_linked {
                vec!["linked".to_string()]
            } else {
                Vec::new()
            },
            open_blockers: blockers
                .iter()
                .filter_map(|uuid| numbers.get(uuid).copied())
                .collect(),
        };
        let state = classify_issue(&github_issue);

        // Find agent for this issue if linked
        let agent = if is_linked {
            // Find link for this issue
            links
                .iter()
                .find(|link| link.issue == issue.id)
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
            blockers,
            linked: is_linked,
            agent,
            number,
        });
    }

    // Sort rows by number (which is already UUID sort order, but keep for safety)
    rows.sort_by_key(|r| r.number);

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
                    .position(|r| r.issue.id == row.issue.id)
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
                    "Read this as: every local issue, grouped by state.",
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
        .extra
        .get("labels")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .filter(|label| *label != "wayfinder:map")
        .collect::<Vec<_>>();

    let mut spans = vec![
        Span::styled(format!("  {} ", row_symbol(row.state)), style),
        Span::styled(format!("#{} ", row.number), style),
        Span::styled(row.issue.title.clone(), style),
    ];
    if !labels.is_empty() {
        spans.push(Span::styled(
            format!("  [{}]", labels.join(", ")),
            if selected {
                style.fg(Color::Black)
            } else {
                style.fg(Color::DarkGray)
            },
        ));
    }
    if !row.blockers.is_empty() {
        let blocker_text = format!(
            "  blocked by {}",
            row.blockers
                .iter()
                .map(|id| id.to_string().chars().take(8).collect::<String>())
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
    use crate::herdr::{AgentStatus, Snapshot};
    use crate::tracker::{Issue, Link, Status};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn test_issue(id: &str, title: &str, status: Status, body: &str) -> Issue {
        Issue {
            id: Uuid::parse_str(id).unwrap(),
            title: title.to_string(),
            status,
            body: body.to_string(),
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
        let id_frontier = "11111111-1111-4111-8111-111111111111";
        let id_blocked = "22222222-2222-4222-8222-222222222222";
        let id_assigned = "33333333-3333-4333-8333-333333333333";
        let id_done = "44444444-4444-4444-8444-444444444444";
        let id_blocker = "55555555-5555-4555-8555-555555555555";

        let blocker = test_issue(id_blocker, "Blocker", Status::Open, "");
        let frontier = test_issue(id_frontier, "Frontier", Status::Open, "");
        let blocked = test_issue(
            id_blocked,
            "Blocked",
            Status::Open,
            &format!("Blocked by: {}", id_blocker),
        );
        let assigned = test_issue(id_assigned, "Assigned", Status::Open, "");
        let done = test_issue(id_done, "Done", Status::Closed, "");

        let link = test_link(
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            id_assigned,
            "/tmp/repo",
        );

        let data = build_issues_data(
            vec![
                frontier.clone(),
                blocked.clone(),
                assigned.clone(),
                done.clone(),
                blocker.clone(),
            ],
            vec![link],
            &Snapshot::default(),
        );

        let grouped = data.grouped();
        let frontier_group = grouped
            .iter()
            .find(|(s, _)| *s == FrontierState::Frontier)
            .unwrap();
        assert!(
            frontier_group
                .1
                .iter()
                .any(|r| r.issue.id == Uuid::parse_str(id_frontier).unwrap())
        );
        // Blocker itself is also frontier (open, unblocked, unassigned) so it will be in frontier too
        let blocked_group = grouped
            .iter()
            .find(|(s, _)| *s == FrontierState::Blocked)
            .unwrap();
        assert!(
            blocked_group
                .1
                .iter()
                .any(|r| r.issue.id == Uuid::parse_str(id_blocked).unwrap())
        );
        let assigned_group = grouped
            .iter()
            .find(|(s, _)| *s == FrontierState::Assigned)
            .unwrap();
        assert!(
            assigned_group
                .1
                .iter()
                .any(|r| r.issue.id == Uuid::parse_str(id_assigned).unwrap())
        );
        let done_group = grouped
            .iter()
            .find(|(s, _)| *s == FrontierState::Done)
            .unwrap();
        assert!(
            done_group
                .1
                .iter()
                .any(|r| r.issue.id == Uuid::parse_str(id_done).unwrap())
        );
    }

    #[test]
    fn renders_grouped_sections() {
        let id1 = "11111111-1111-4111-8111-111111111111";
        let id2 = "22222222-2222-4222-8222-222222222222";
        let issues = vec![
            test_issue(id1, "Frontier Issue", Status::Open, ""),
            test_issue(id2, "Done Issue", Status::Closed, ""),
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
        let id1 = "11111111-1111-4111-8111-111111111111";
        let issues = vec![test_issue(id1, "Issue", Status::Open, "")];
        let data = build_issues_data(issues, vec![], &Snapshot::default());
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        IssuesWidget {
            data: &data,
            selected: Some(0),
        }
        .render(area, &mut buf);
        // Selected row is at y=2 (header 0, blank 1, Frontier header 2, row 3?) Actually ISSUES header at y0, Frontier header at y1, row at y2
        // Find row with Issue
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
