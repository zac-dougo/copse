#![allow(dead_code)]

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::discovery::Worktree;
use crate::forest::branch_is_main;
use crate::herdr::AgentStatus;

pub struct MapWidget<'a> {
    pub worktrees: &'a [Worktree],
    pub agents: &'a [crate::herdr::Agent],
}

impl<'a> Widget for MapWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let mut y = area.y;
        let max_y = area.y + area.height;

        // Left: WORKTREES
        if y < max_y {
            let line = Line::from(vec![Span::styled(
                "WORKTREES",
                Style::default().add_modifier(Modifier::BOLD),
            )]);
            buf.set_line(area.x + 2, y, &line, area.width);
            y += 1;
        }
        // Worktree list with markers
        for wt in self.worktrees {
            if y >= max_y {
                break;
            }
            // Find agent for this worktree to get status marker
            let mut marker = "·";
            let mut color = Color::DarkGray;
            let mut is_main = branch_is_main(&wt.branch);
            // Find matching agent by prefix
            if let Some(agent) = self.agents.iter().find(|a| {
                let cwd = a.foreground_cwd.as_ref().or(a.cwd.as_ref());
                if let Some(cwd) = cwd {
                    std::path::Path::new(cwd).starts_with(&wt.path)
                } else {
                    false
                }
            }) {
                match agent.agent_status {
                    AgentStatus::Working => {
                        marker = "●";
                        color = Color::Blue;
                    }
                    AgentStatus::Done => {
                        marker = "✓";
                        color = Color::Green;
                    }
                    AgentStatus::Blocked => {
                        marker = "!";
                        color = Color::Red;
                    }
                    AgentStatus::Idle => {
                        marker = "○";
                        color = Color::Green;
                    }
                    AgentStatus::Unknown => {
                        marker = "·";
                        color = Color::DarkGray;
                    }
                }
                is_main = is_main
                    || wt
                        .branch
                        .as_deref()
                        .map(|b| b.contains("main"))
                        .unwrap_or(false);
            }
            let branch_name = wt
                .branch
                .as_ref()
                .map(|b| b.strip_prefix("refs/heads/").unwrap_or(b).to_string())
                .unwrap_or_else(|| wt.path.display().to_string());
            let style = if is_main {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            let line = Line::from(vec![
                Span::styled(format!("{marker} "), style),
                Span::styled(branch_name, style),
            ]);
            buf.set_line(area.x + 2, y, &line, area.width);
            y += 1;
        }

        // Right side: Wayfinder placeholder - render at same y start but offset
        // For simplicity, we render wayfinder below worktrees if area is tall enough,
        // or we could split horizontally. For v1, we render below with header.
        if y + 2 < max_y {
            y += 1;
            let header = Line::from(vec![Span::styled(
                "WAYFINDER / COPSE V1",
                Style::default().add_modifier(Modifier::BOLD),
            )]);
            buf.set_line(area.x + 2, y, &header, area.width);
            y += 1;
            // Simple dependency diagram similar to prototype
            let lines = [
                "[Research Herdr observation] ──┐",
                "[Research terminal app options] ─┼─> [Choose terminal app stack]",
                "[Prototype board visualization] ──┘              │",
                "                                              ┌──┴──┐",
                "                                   [Define board interaction]",
                "",
                "[Decide issue write boundary] ─> [Define local tracker schema] ─┘",
                "",
                "open 6   blocked 2   resolved 2",
            ];
            for l in lines {
                if y >= max_y {
                    break;
                }
                let line = Line::from(vec![Span::styled(
                    l.to_string(),
                    Style::default().fg(Color::DarkGray),
                )]);
                buf.set_line(area.x + 2, y, &line, area.width);
                y += 1;
            }
        }

        if y + 1 < max_y {
            let footer = Line::from(vec![Span::styled(
                "Read this as: which decision unlocks the next move. Worktrees stay visible as live context.",
                Style::default().fg(Color::DarkGray),
            )]);
            buf.set_line(area.x + 2, y + 1, &footer, area.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::Worktree;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use std::path::PathBuf;

    fn wt(path: &str, branch: &str) -> Worktree {
        Worktree {
            path: PathBuf::from(path),
            head: "abc".to_string(),
            branch: Some(format!("refs/heads/{branch}")),
            is_bare: false,
            is_detached: false,
            is_prunable: false,
        }
    }

    #[test]
    fn renders_worktrees_and_wayfinder() {
        let wts = vec![wt("/tmp/repo", "main"), wt("/tmp/feature", "feature/a")];
        let agents = vec![];
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        MapWidget {
            worktrees: &wts,
            agents: &agents,
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
        assert!(content.contains("WORKTREES"));
        assert!(content.contains("main"));
        assert!(content.contains("WAYFINDER"));
        assert!(content.contains("Research Herdr observation"));
    }
}
