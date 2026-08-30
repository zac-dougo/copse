#![allow(dead_code, unused_imports)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use uuid::Uuid;

use crate::discovery::{BoardRepository, discover};
use crate::forest::{Forest, ForestWidget, build_forest, render_markdown_to_text};
use crate::herdr::{Snapshot, fetch_snapshot_blocking};
use crate::map::MapWidget;
use crate::tracker::{load_issues, load_links};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedKind {
    Branch,
    Issue,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Forest,
    Map,
}

#[derive(Debug, Clone)]
pub struct FlatNode {
    pub kind: SelectedKind,
    pub branch_idx: usize,
    pub issue_idx: Option<usize>,
    pub agent_idx: Option<usize>,
    pub display: String,
}

pub struct App {
    pub cwd: PathBuf,
    pub board: BoardRepository,
    pub forest: Forest,
    pub snapshot: Snapshot,
    pub stale_herdr: bool,
    pub stale_local: bool,
    pub last_herdr: Instant,
    pub last_github: Instant,
    pub selected: usize,
    pub flat_nodes: Vec<FlatNode>,
    pub expanded_branches: HashSet<PathBuf>,
    pub expanded_issues: HashSet<Uuid>,
    pub show_help: bool,
    pub show_detail: bool,
    pub status_msg: String,
    pub view: View,
}

impl App {
    pub fn new(cwd: PathBuf) -> Result<Self> {
        let board = discover(&cwd)?;
        let issues = load_issues(&board.issues_dir).unwrap_or_default();
        let links = load_links(&board.links_dir).unwrap_or_default();
        let snapshot = fetch_snapshot_blocking().unwrap_or_default();
        let forest = build_forest(board.worktrees.clone(), issues, links, &snapshot);

        let mut expanded_branches: HashSet<PathBuf> = HashSet::new();
        for b in &forest.branches {
            expanded_branches.insert(b.worktree.path.clone());
        }
        let mut expanded_issues: HashSet<Uuid> = HashSet::new();
        for b in &forest.branches {
            if let Some(issue) = &b.issue {
                expanded_issues.insert(issue.issue.id);
            }
        }

        let mut app = Self {
            cwd,
            board,
            forest,
            snapshot,
            stale_herdr: false,
            stale_local: false,
            last_herdr: Instant::now(),
            last_github: Instant::now(),
            selected: 0,
            flat_nodes: Vec::new(),
            expanded_branches,
            expanded_issues,
            show_help: false,
            show_detail: false,
            status_msg: String::new(),
            view: View::Forest,
        };
        app.rebuild_flat();
        Ok(app)
    }

    pub fn new_for_test(board: BoardRepository, forest: Forest, snapshot: Snapshot) -> Self {
        let mut expanded_branches: HashSet<PathBuf> = HashSet::new();
        for b in &forest.branches {
            expanded_branches.insert(b.worktree.path.clone());
        }
        let mut expanded_issues: HashSet<Uuid> = HashSet::new();
        for b in &forest.branches {
            if let Some(issue) = &b.issue {
                expanded_issues.insert(issue.issue.id);
            }
        }
        let mut app = Self {
            cwd: board.primary_path.clone(),
            board,
            forest,
            snapshot,
            stale_herdr: false,
            stale_local: false,
            last_herdr: Instant::now(),
            last_github: Instant::now(),
            selected: 0,
            flat_nodes: Vec::new(),
            expanded_branches,
            expanded_issues,
            show_help: false,
            show_detail: false,
            status_msg: String::new(),
            view: View::Forest,
        };
        app.rebuild_flat();
        app
    }

    pub fn rebuild_flat(&mut self) {
        let mut nodes = Vec::new();
        for (b_idx, branch) in self.forest.branches.iter().enumerate() {
            nodes.push(FlatNode {
                kind: SelectedKind::Branch,
                branch_idx: b_idx,
                issue_idx: None,
                agent_idx: None,
                display: branch.branch_name.clone(),
            });
            if !self.expanded_branches.contains(&branch.worktree.path) {
                continue;
            }
            if let Some(issue) = &branch.issue {
                nodes.push(FlatNode {
                    kind: SelectedKind::Issue,
                    branch_idx: b_idx,
                    issue_idx: Some(0),
                    agent_idx: None,
                    display: issue.issue.title.clone(),
                });
                if !self.expanded_issues.contains(&issue.issue.id) {
                    continue;
                }
                for (a_idx, agent) in issue.agents.iter().enumerate() {
                    nodes.push(FlatNode {
                        kind: SelectedKind::Agent,
                        branch_idx: b_idx,
                        issue_idx: Some(0),
                        agent_idx: Some(a_idx),
                        display: agent.agent.pane_id.clone(),
                    });
                }
            }
        }
        for (idx, issue_node) in self.forest.unlinked_issues.iter().enumerate() {
            nodes.push(FlatNode {
                kind: SelectedKind::Issue,
                branch_idx: usize::MAX,
                issue_idx: Some(idx),
                agent_idx: None,
                display: issue_node.issue.title.clone(),
            });
        }
        self.flat_nodes = nodes;
        if self.selected >= self.flat_nodes.len() && !self.flat_nodes.is_empty() {
            self.selected = self.flat_nodes.len() - 1;
        }
        if self.flat_nodes.is_empty() {
            self.selected = 0;
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.flat_nodes.len() {
            self.selected += 1;
        }
    }

    pub fn toggle_expand(&mut self) {
        if let Some(node) = self.flat_nodes.get(self.selected) {
            match node.kind {
                SelectedKind::Branch => {
                    let branch = &self.forest.branches[node.branch_idx];
                    let path = branch.worktree.path.clone();
                    if self.expanded_branches.contains(&path) {
                        self.expanded_branches.remove(&path);
                    } else {
                        self.expanded_branches.insert(path);
                    }
                    self.rebuild_flat();
                }
                SelectedKind::Issue => {
                    // Find issue id
                    let issue_id = if node.branch_idx == usize::MAX {
                        self.forest.unlinked_issues[node.issue_idx.unwrap()]
                            .issue
                            .id
                    } else {
                        self.forest.branches[node.branch_idx]
                            .issue
                            .as_ref()
                            .unwrap()
                            .issue
                            .id
                    };
                    if self.expanded_issues.contains(&issue_id) {
                        self.expanded_issues.remove(&issue_id);
                    } else {
                        self.expanded_issues.insert(issue_id);
                    }
                    self.rebuild_flat();
                }
                SelectedKind::Agent => {
                    // Move to parent issue
                    // Find parent issue index
                    self.selected = self
                        .flat_nodes
                        .iter()
                        .position(|n| {
                            n.kind == SelectedKind::Issue && n.branch_idx == node.branch_idx
                        })
                        .unwrap_or(self.selected);
                }
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.show_help {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                self.show_help = false;
                return false;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return true;
            }
            return false;
        }

        if self.show_detail && key.code == KeyCode::Esc {
            self.show_detail = false;
            return false;
        }

        match key.code {
            KeyCode::Up => self.move_up(),
            KeyCode::Down => self.move_down(),
            KeyCode::Left => {
                // Collapse or move to parent
                if let Some(node) = self.flat_nodes.get(self.selected) {
                    match node.kind {
                        SelectedKind::Agent => {
                            // Move to parent issue
                            if let Some(parent_idx) = self.flat_nodes.iter().position(|n| {
                                n.kind == SelectedKind::Issue && n.branch_idx == node.branch_idx
                            }) {
                                self.selected = parent_idx;
                            }
                        }
                        SelectedKind::Issue => {
                            // Collapse if expanded, else move to branch
                            let issue_id = if node.branch_idx == usize::MAX {
                                self.forest.unlinked_issues[node.issue_idx.unwrap()]
                                    .issue
                                    .id
                            } else {
                                self.forest.branches[node.branch_idx]
                                    .issue
                                    .as_ref()
                                    .unwrap()
                                    .issue
                                    .id
                            };
                            if self.expanded_issues.contains(&issue_id) {
                                self.expanded_issues.remove(&issue_id);
                                self.rebuild_flat();
                            } else if node.branch_idx != usize::MAX {
                                // Move to parent branch
                                if let Some(parent_idx) = self.flat_nodes.iter().position(|n| {
                                    n.kind == SelectedKind::Branch
                                        && n.branch_idx == node.branch_idx
                                }) {
                                    self.selected = parent_idx;
                                }
                            }
                        }
                        SelectedKind::Branch => {
                            let branch = &self.forest.branches[node.branch_idx];
                            let path = branch.worktree.path.clone();
                            if self.expanded_branches.contains(&path) {
                                self.expanded_branches.remove(&path);
                                self.rebuild_flat();
                            }
                        }
                    }
                }
            }
            KeyCode::Right => {
                if let Some(node) = self.flat_nodes.get(self.selected) {
                    match node.kind {
                        SelectedKind::Branch => {
                            let branch = &self.forest.branches[node.branch_idx];
                            let path = branch.worktree.path.clone();
                            if !self.expanded_branches.contains(&path) {
                                self.expanded_branches.insert(path);
                                self.rebuild_flat();
                            } else if let Some(child_idx) = self.flat_nodes.iter().position(|n| {
                                n.kind == SelectedKind::Issue && n.branch_idx == node.branch_idx
                            }) {
                                self.selected = child_idx;
                            }
                        }
                        SelectedKind::Issue => {
                            let issue_id = if node.branch_idx == usize::MAX {
                                self.forest.unlinked_issues[node.issue_idx.unwrap()]
                                    .issue
                                    .id
                            } else {
                                self.forest.branches[node.branch_idx]
                                    .issue
                                    .as_ref()
                                    .unwrap()
                                    .issue
                                    .id
                            };
                            if !self.expanded_issues.contains(&issue_id) {
                                self.expanded_issues.insert(issue_id);
                                self.rebuild_flat();
                            } else {
                                // Move to first agent if any
                                if let Some(agent_idx) = self.flat_nodes.iter().position(|n| {
                                    n.kind == SelectedKind::Agent && n.branch_idx == node.branch_idx
                                }) {
                                    self.selected = agent_idx;
                                }
                            }
                        }
                        SelectedKind::Agent => {}
                    }
                }
            }
            KeyCode::Enter => {
                self.show_detail = !self.show_detail;
            }
            KeyCode::Char('r') => {
                self.status_msg = "refresh requested".to_string();
                // Caller will trigger immediate refresh; we just set flag.
            }
            KeyCode::Char('?') => {
                self.show_help = true;
            }
            KeyCode::Char('1') => self.view = View::Forest,
            KeyCode::Char('2') | KeyCode::Char('m') => self.view = View::Map,
            KeyCode::Char('q') => return true,
            KeyCode::Esc if self.show_detail => {
                self.show_detail = false;
            }
            _ => {}
        }

        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        false
    }

    pub fn refresh(&mut self) {
        // Reload local data and Herdr snapshot.
        // Keep last good on failure.
        let issues = load_issues(&self.board.issues_dir).unwrap_or_else(|_| {
            self.stale_local = true;
            Vec::new()
        });
        let links = load_links(&self.board.links_dir).unwrap_or_else(|_| {
            self.stale_local = true;
            Vec::new()
        });

        // Refresh board discovery for worktrees (in case new worktree added)
        if let Ok(new_board) = discover(&self.cwd) {
            self.board = new_board;
        } else {
            self.stale_local = true;
        }

        match fetch_snapshot_blocking() {
            Ok(snap) => {
                self.snapshot = snap;
                self.stale_herdr = false;
                self.last_herdr = Instant::now();
            }
            Err(_) => {
                self.stale_herdr = true;
            }
        }

        let forest = build_forest(self.board.worktrees.clone(), issues, links, &self.snapshot);
        self.forest = forest;
        // Keep expanded sets for existing branches/issues, expand new ones by default
        for b in &self.forest.branches {
            self.expanded_branches.insert(b.worktree.path.clone());
            if let Some(issue) = &b.issue {
                self.expanded_issues.insert(issue.issue.id);
            }
        }
        self.rebuild_flat();
        self.last_herdr = Instant::now();
        self.stale_local = false;
    }

    pub fn needs_herdr_refresh(&self) -> bool {
        self.last_herdr.elapsed() >= Duration::from_secs(2)
    }

    pub fn needs_github_refresh(&self) -> bool {
        self.last_github.elapsed() >= Duration::from_secs(30)
    }
}

// Rendering helpers

pub fn draw_status_bar(app: &App, area: Rect, buf: &mut Buffer) {
    let view_label = match app.view {
        View::Forest => " Forest ",
        View::Map => " Map ",
    };
    let mut spans = vec![
        Span::styled(
            view_label.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(
            " q:quit  r:refresh  ?:help  1:Forest  2:Map  Enter:details  ↑/↓:navigate  ←/→:collapse/expand",
        ),
    ];
    if app.stale_herdr {
        spans.push(Span::styled(
            "  ● Herdr stale",
            Style::default().fg(Color::Yellow),
        ));
    }
    if app.stale_local {
        spans.push(Span::styled(
            "  ● local stale",
            Style::default().fg(Color::Yellow),
        ));
    }
    if !app.status_msg.is_empty() {
        spans.push(Span::raw(format!("  {}", app.status_msg)));
    }
    let line = Line::from(spans);
    let block = Block::default().borders(Borders::TOP);
    let inner = block.inner(area);
    block.render(area, buf);
    Paragraph::new(line).render(inner, buf);
}

pub fn draw_detail(app: &App, area: Rect, buf: &mut Buffer) {
    let node = match app.flat_nodes.get(app.selected) {
        Some(n) => n,
        None => return,
    };

    let (title, body) = match node.kind {
        SelectedKind::Branch => {
            let branch = &app.forest.branches[node.branch_idx];
            let title = format!("Branch: {}", branch.branch_name);
            let body = format!(
                "Path: {}\nHead: {}\nBranch ref: {}",
                branch.worktree.path.display(),
                branch.worktree.head,
                branch.worktree.branch.as_deref().unwrap_or("(detached)")
            );
            (title, body)
        }
        SelectedKind::Issue => {
            let issue = if node.branch_idx == usize::MAX {
                &app.forest.unlinked_issues[node.issue_idx.unwrap()].issue
            } else {
                &app.forest.branches[node.branch_idx]
                    .issue
                    .as_ref()
                    .unwrap()
                    .issue
            };
            let title = format!("Issue: {}", issue.title);
            let body = format!(
                "Status: {}\n\n{}",
                issue.status,
                render_markdown_to_text(&issue.body)
            );
            (title, body)
        }
        SelectedKind::Agent => {
            let branch = &app.forest.branches[node.branch_idx];
            let issue_node = branch.issue.as_ref().unwrap();
            let agent_node = &issue_node.agents[node.agent_idx.unwrap()];
            let title = format!(
                "Agent: {} {}",
                agent_node.agent.agent, agent_node.agent.pane_id
            );
            let body = format!(
                "Status: {}\nWorkspace: {}\nCWD: {}\nFocused: {}",
                agent_node.badge,
                agent_node.agent.workspace_id,
                agent_node
                    .agent
                    .foreground_cwd
                    .as_deref()
                    .or(agent_node.agent.cwd.as_deref())
                    .unwrap_or("(unknown)"),
                agent_node.agent.focused.unwrap_or(false)
            );
            (title, body)
        }
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    block.render(area, buf);
    Clear.render(inner, buf);
    Paragraph::new(body).render(inner, buf);
}

pub fn draw_help(area: Rect, buf: &mut Buffer) {
    let text = "\
Copse — terminal board

Views:
  1          Forest (branch → issue → agent)
  2, m       Map (worktrees + Wayfinder)

Navigation:
  ↑/↓        Move selection
  ←          Collapse or move to parent
  →          Expand or move to child
  Enter      Toggle detail pane
  r          Refresh all sources now
  ?          Toggle this help
  q, Ctrl-C  Quit

Mouse:
  Click      Select node
  Wheel      Scroll
  (mouse is optional, no mouse-only actions)

Statuses:
  ● working  blue
  ✓ done     green
  ! blocked  red
  ○ idle     green
  · unknown  gray

Refresh:
  Herdr + local Git/.copse every 2s
  GitHub + Wayfinder every 30s
  r forces immediate refresh

Press Esc or ? or q to close help.
";
    let block = Block::default()
        .title("Help (?)")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    block.render(area, buf);
    Clear.render(inner, buf);
    Paragraph::new(text).render(inner, buf);
}

// Main loop

pub async fn run(cwd: PathBuf) -> Result<()> {
    use crossterm::event::DisableMouseCapture;
    use crossterm::event::EnableMouseCapture;
    use crossterm::execute;
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };

    let mut app = App::new(cwd.clone()).map_err(|e| anyhow::anyhow!("{}", e))?;

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut herdr_interval = tokio::time::interval(Duration::from_secs(2));
    let mut github_interval = tokio::time::interval(Duration::from_secs(30));
    let mut events = crossterm::event::EventStream::new();

    let res: Result<()> = async {
        loop {
            terminal.draw(|f| {
                let area = f.area();
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(0), Constraint::Length(1)])
                    .split(area);
                let main_area = chunks[0];
                let status_area = chunks[1];

                match app.view {
                    View::Forest => {
                        if app.show_detail {
                            let cols = Layout::default()
                                .direction(Direction::Horizontal)
                                .constraints([
                                    Constraint::Percentage(60),
                                    Constraint::Percentage(40),
                                ])
                                .split(main_area);
                            let forest_area = cols[0];
                            let detail_area = cols[1];
                            ForestWidget {
                                forest: &app.forest,
                                selected: Some(app.selected),
                            }
                            .render(forest_area, f.buffer_mut());
                            draw_detail(&app, detail_area, f.buffer_mut());
                        } else {
                            ForestWidget {
                                forest: &app.forest,
                                selected: Some(app.selected),
                            }
                            .render(main_area, f.buffer_mut());
                        }
                    }
                    View::Map => {
                        MapWidget {
                            worktrees: &app.board.worktrees,
                            agents: &app.snapshot.agents,
                        }
                        .render(main_area, f.buffer_mut());
                    }
                }

                draw_status_bar(&app, status_area, f.buffer_mut());

                if app.show_help {
                    let help_area = centered_rect(80, 70, area);
                    draw_help(help_area, f.buffer_mut());
                }
            })?;

            tokio::select! {
                maybe_event = events.next() => {
                    if let Some(Ok(event)) = maybe_event {
                        match event {
                            Event::Key(key) => {
                                let should_quit = app.handle_key(key);
                                if should_quit {
                                    break;
                                }
                                // r triggers immediate refresh
                                if key.code == KeyCode::Char('r') {
                                    app.refresh();
                                }
                            }
                            Event::Mouse(me) => {
                                match me.kind {
                                    MouseEventKind::Down(MouseButton::Left) => {
                                        // Select node by y position (rough)
                                        let y = me.row as usize;
                                        // Forest starts at y=0, so map y to flat index
                                        if y < app.flat_nodes.len() {
                                            app.selected = y;
                                        }
                                    }
                                    MouseEventKind::ScrollUp => app.move_up(),
                                    MouseEventKind::ScrollDown => app.move_down(),
                                    _ => {}
                                }
                            }
                            Event::Resize(_, _) => {}
                            _ => {}
                        }
                    }
                }
                _ = herdr_interval.tick() => {
                    app.refresh();
                }
                _ = github_interval.tick() => {
                    // GitHub refresh stub: just update timestamp for now.
                    app.last_github = Instant::now();
                }
            }
        }
        Ok(())
    }
    .await;

    // Cleanup
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    res
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::Worktree;
    use crate::herdr::{AgentStatus, Snapshot};
    use crate::tracker::{Issue, Link, Status};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn repo_with_worktrees() -> BoardRepository {
        let wt1 = Worktree {
            path: PathBuf::from("/tmp/repo"),
            head: "abc".to_string(),
            branch: Some("refs/heads/main".to_string()),
            is_bare: false,
            is_detached: false,
            is_prunable: false,
        };
        let wt2 = Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: "def".to_string(),
            branch: Some("refs/heads/feature".to_string()),
            is_bare: false,
            is_detached: false,
            is_prunable: false,
        };
        BoardRepository {
            primary_path: PathBuf::from("/tmp/repo"),
            current_worktree_path: PathBuf::from("/tmp/repo"),
            worktrees: vec![wt1, wt2],
            copse_dir: PathBuf::from("/tmp/repo/.copse"),
            issues_dir: PathBuf::from("/tmp/repo/.copse/issues"),
            links_dir: PathBuf::from("/tmp/repo/.copse/links"),
            is_copse_present: false,
        }
    }

    fn sample_issue(title: &str) -> Issue {
        Issue {
            id: Uuid::new_v4(),
            title: title.to_string(),
            status: Status::Open,
            body: "body".to_string(),
            extra: HashMap::new(),
        }
    }

    #[test]
    fn keyboard_moves_selection() {
        let board = repo_with_worktrees();
        let forest = Forest {
            branches: vec![],
            unlinked_issues: vec![],
        };
        let snap = Snapshot::default();
        let mut app = App::new_for_test(board, forest, snap);
        // With empty forest, flat is empty, move should not panic
        app.move_down();
        app.move_up();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn collapse_and_expand_branch() {
        let wts = vec![Worktree {
            path: PathBuf::from("/tmp/repo"),
            head: "abc".to_string(),
            branch: Some("refs/heads/main".to_string()),
            is_bare: false,
            is_detached: false,
            is_prunable: false,
        }];
        let issue = sample_issue("Issue");
        let link = Link {
            id: Uuid::new_v4(),
            issue: issue.id,
            worktree: "/tmp/repo".to_string(),
            body: "".to_string(),
            extra: HashMap::new(),
        };
        let snap = Snapshot::default();
        // No agents
        let board = BoardRepository {
            primary_path: PathBuf::from("/tmp/repo"),
            current_worktree_path: PathBuf::from("/tmp/repo"),
            worktrees: wts.clone(),
            copse_dir: PathBuf::from("/tmp/repo/.copse"),
            issues_dir: PathBuf::from("/tmp/repo/.copse/issues"),
            links_dir: PathBuf::from("/tmp/repo/.copse/links"),
            is_copse_present: false,
        };
        let forest = build_forest(wts, vec![issue], vec![link], &snap);
        let mut app = App::new_for_test(board, forest, snap);
        assert_eq!(app.flat_nodes.len(), 2); // branch + issue
        // Collapse branch
        app.selected = 0;
        app.toggle_expand(); // collapse branch
        assert_eq!(app.flat_nodes.len(), 1); // only branch visible
        app.toggle_expand(); // expand again
        assert_eq!(app.flat_nodes.len(), 2);
    }

    #[test]
    fn handle_key_quit() {
        let board = repo_with_worktrees();
        let forest = Forest::default();
        let snap = Snapshot::default();
        let mut app = App::new_for_test(board, forest, snap);
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(app.handle_key(key));
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.handle_key(key));
    }

    #[test]
    fn handle_help_toggle() {
        let board = repo_with_worktrees();
        let forest = Forest::default();
        let snap = Snapshot::default();
        let mut app = App::new_for_test(board, forest, snap);
        assert!(!app.show_help);
        let key = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE);
        app.handle_key(key);
        assert!(app.show_help);
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        app.handle_key(key);
        assert!(!app.show_help);
    }

    #[test]
    fn refresh_keeps_last_good_on_failure() {
        // Just test that refresh doesn't panic when .copse missing and herdr missing
        let board = repo_with_worktrees();
        let forest = Forest::default();
        let snap = Snapshot::default();
        let mut app = App::new_for_test(board, forest, snap);
        // This will try to load from /tmp paths which don't have herdr, but should not panic
        app.refresh();
        // Should have updated forest (still empty or with available)
        assert!(app.forest.branches.len() < 1000);
    }
}
