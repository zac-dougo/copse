#![allow(dead_code)]

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::github::{FrontierState, MapData, WayfinderChild, WayfinderMap};

pub struct MapWidget<'a> {
    pub maps: &'a MapData,
    pub selected_map: usize,
    pub selected_child: Option<usize>,
}

impl<'a> Widget for MapWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let max_y = area.y + area.height;
        let footer_y = max_y - 1;
        let mut y = area.y;

        write_line(
            buf,
            area.x + 2,
            y,
            area.width.saturating_sub(2),
            Line::from(Span::styled(
                "WAYFINDER",
                Style::default().add_modifier(Modifier::BOLD),
            )),
        );
        y += 1;

        if let Some(map) = self.maps.get(self.selected_map) {
            y = render_map_header(map, area, y, footer_y, buf);
            render_sections(map, area, y, footer_y, self.selected_child, buf);
        } else if y < footer_y {
            write_line(
                buf,
                area.x + 2,
                y,
                area.width.saturating_sub(2),
                Line::from(Span::styled(
                    "No Wayfinder maps found.",
                    Style::default().fg(Color::DarkGray),
                )),
            );
        }

        if footer_y > area.y {
            write_line(
                buf,
                area.x + 2,
                footer_y,
                area.width.saturating_sub(2),
                Line::from(Span::styled(
                    "Read this as: frontier work first. Blocked work names its blockers.",
                    Style::default().fg(Color::DarkGray),
                )),
            );
        }
    }
}

fn render_map_header(
    map: &WayfinderMap,
    area: Rect,
    mut y: u16,
    footer_y: u16,
    buf: &mut Buffer,
) -> u16 {
    if y >= footer_y {
        return y;
    }
    let header = format!(
        "#{number} {title}  ({open} open / {total} issues)",
        number = map.issue.number,
        title = map.issue.title,
        open = map.open_child_count,
        total = map.children.len(),
    );
    write_line(
        buf,
        area.x + 2,
        y,
        area.width.saturating_sub(2),
        Line::from(Span::styled(
            header,
            Style::default().add_modifier(Modifier::BOLD),
        )),
    );
    y += 1;
    if y < footer_y {
        write_line(
            buf,
            area.x + 2,
            y,
            area.width.saturating_sub(2),
            Line::from(Span::styled(
                "Tab: next map",
                Style::default().fg(Color::DarkGray),
            )),
        );
        y += 1;
    }
    y
}

fn render_sections(
    map: &WayfinderMap,
    area: Rect,
    mut y: u16,
    footer_y: u16,
    selected_child: Option<usize>,
    buf: &mut Buffer,
) {
    for state in [
        FrontierState::Frontier,
        FrontierState::Blocked,
        FrontierState::Assigned,
        FrontierState::Done,
    ] {
        if y >= footer_y {
            break;
        }
        let children = map
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| child.state == state)
            .collect::<Vec<_>>();
        let (label_color, label) = section_label(state);
        write_line(
            buf,
            area.x + 2,
            y,
            area.width.saturating_sub(2),
            Line::from(Span::styled(
                format!("─ {label} ({})", children.len()),
                Style::default().fg(label_color),
            )),
        );
        y += 1;

        if children.is_empty() {
            if y < footer_y {
                write_line(
                    buf,
                    area.x + 2,
                    y,
                    area.width.saturating_sub(2),
                    Line::from(Span::styled(
                        "  (none)",
                        Style::default().fg(Color::DarkGray),
                    )),
                );
                y += 1;
            }
            continue;
        }

        for (index, child) in children {
            if y >= footer_y {
                break;
            }
            write_child(area, y, child, selected_child == Some(index), buf);
            y += 1;
        }
    }
}

fn write_child(area: Rect, y: u16, child: &WayfinderChild, selected: bool, buf: &mut Buffer) {
    let color = child_color(child.state);
    let style = if selected {
        Style::default()
            .fg(color)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    let label = child.issue.labels.iter().find_map(|label| {
        label
            .strip_prefix("wayfinder:")
            .filter(|name| *name != "map")
    });
    let blocker_text = if child.issue.open_blockers.is_empty() {
        None
    } else {
        Some(format!(
            "  blocked by {}",
            child
                .issue
                .open_blockers
                .iter()
                .map(|number| format!("#{number}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    };

    let mut spans = vec![
        Span::styled(format!("  {} ", child_symbol(child.state)), style),
        Span::styled(format!("#{} ", child.issue.number), style),
        Span::styled(child.issue.title.clone(), style),
    ];
    if let Some(label) = label {
        spans.push(Span::styled(
            format!("  [{label}]"),
            style.fg(Color::DarkGray),
        ));
    }
    if let Some(blocker_text) = blocker_text {
        spans.push(Span::styled(blocker_text, style.fg(Color::Red)));
    }

    write_line(
        buf,
        area.x + 2,
        y,
        area.width.saturating_sub(2),
        Line::from(spans),
    );
}

fn write_line(buf: &mut Buffer, x: u16, y: u16, width: u16, line: Line<'static>) {
    buf.set_line(x, y, &line, width);
}

fn section_label(state: FrontierState) -> (Color, &'static str) {
    match state {
        FrontierState::Frontier => (Color::Green, "Frontier"),
        FrontierState::Blocked => (Color::Red, "Blocked"),
        FrontierState::Assigned => (Color::Cyan, "Assigned"),
        FrontierState::Done => (Color::DarkGray, "Done"),
    }
}

fn child_color(state: FrontierState) -> Color {
    match state {
        FrontierState::Frontier => Color::Green,
        FrontierState::Blocked => Color::Red,
        FrontierState::Assigned => Color::Cyan,
        FrontierState::Done => Color::Gray,
    }
}

fn child_symbol(state: FrontierState) -> &'static str {
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
    use crate::github::{GitHubIssue, IssueState};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn child(number: u64, title: &str, state: FrontierState) -> WayfinderChild {
        WayfinderChild {
            issue: GitHubIssue {
                number,
                title: title.to_string(),
                state: if state == FrontierState::Done {
                    IssueState::Closed
                } else {
                    IssueState::Open
                },
                body: String::new(),
                labels: Vec::new(),
                assignees: Vec::new(),
                open_blockers: if state == FrontierState::Blocked {
                    vec![4]
                } else {
                    Vec::new()
                },
            },
            state,
        }
    }

    fn map() -> WayfinderMap {
        WayfinderMap {
            issue: GitHubIssue {
                number: 10,
                title: "Copse map".to_string(),
                state: IssueState::Open,
                body: String::new(),
                labels: vec!["wayfinder:map".to_string()],
                assignees: Vec::new(),
                open_blockers: Vec::new(),
            },
            children: vec![
                child(1, "Done issue", FrontierState::Done),
                child(2, "Frontier issue", FrontierState::Frontier),
                child(3, "Assigned issue", FrontierState::Assigned),
                child(5, "Blocked issue", FrontierState::Blocked),
            ],
            open_child_count: 3,
        }
    }

    fn content(buf: &Buffer, area: Rect) -> String {
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_frontier_sections_in_display_order() {
        let maps = vec![map()];
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        MapWidget {
            maps: &maps,
            selected_map: 0,
            selected_child: None,
        }
        .render(area, &mut buf);

        let rendered = content(&buf, area);
        assert!(rendered.contains("WAYFINDER"));
        assert!(rendered.contains("#10 Copse map"));
        assert!(rendered.contains("─ Frontier (1)"));
        assert!(rendered.contains("─ Blocked (1)"));
        assert!(rendered.contains("─ Assigned (1)"));
        assert!(rendered.contains("─ Done (1)"));
        assert!(rendered.contains("#2 Frontier issue"));
        assert!(rendered.contains("#5 Blocked issue  blocked by #4"));
        assert!(!rendered.contains("WORKTREES"));
        assert!(!rendered.contains("agent"));
        assert!(rendered.find("Frontier issue") < rendered.find("Blocked issue"));
        assert!(rendered.find("Blocked issue") < rendered.find("Assigned issue"));
        assert!(rendered.find("Assigned issue") < rendered.find("Done issue"));
    }

    #[test]
    fn selected_child_gets_a_background() {
        let maps = vec![map()];
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        MapWidget {
            maps: &maps,
            selected_map: 0,
            selected_child: Some(1),
        }
        .render(area, &mut buf);

        let y = (0..area.height)
            .find(|y| {
                (0..area.width)
                    .map(|x| buf[(x, *y)].symbol().to_string())
                    .collect::<String>()
                    .contains("#2 Frontier issue")
            })
            .unwrap();
        assert_eq!(buf[(2, y)].bg, Color::DarkGray);
    }

    #[test]
    fn empty_map_data_has_an_empty_state() {
        let maps = MapData::new();
        let area = Rect::new(0, 0, 60, 8);
        let mut buf = Buffer::empty(area);
        MapWidget {
            maps: &maps,
            selected_map: 0,
            selected_child: None,
        }
        .render(area, &mut buf);
        assert!(content(&buf, area).contains("No Wayfinder maps found."));
    }
}
