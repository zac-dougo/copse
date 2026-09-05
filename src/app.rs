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
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};
use uuid::Uuid;

use crate::discovery::{BoardRepository, discover};
use crate::forest::{Forest, ForestWidget, build_forest, render_markdown_to_text};
use crate::github::{
    GitHubIssue, MapData, default_map_index, fetch_github_board, map_index_for_number,
};
use crate::herdr::{Snapshot, fetch_snapshot_blocking};
use crate::issues::{IssuesData, IssuesWidget, build_issues_data};
use crate::map::MapWidget;
use crate::tracker::{Link, load_links};

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
    Issues,
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
    pub last_herdr: Instant,
    pub last_github: Instant,
    pub selected: usize,
    pub flat_nodes: Vec<FlatNode>,
    pub expanded_branches: HashSet<PathBuf>,
    pub expanded_issues: HashSet<u64>,
    pub show_help: bool,
    pub show_detail: bool,
    pub status_msg: String,
    pub view: View,
    pub map_data: MapData,
    pub selected_map: usize,
    pub selected_map_child: usize,
    pub map_detail_scroll: u16,
    pub stale_github: bool,
    pub github_issues: Vec<GitHubIssue>,
    pub issues_data: IssuesData,
    pub selected_issue: usize,
    pub issues_detail_scroll: u16,
}

impl App {
    pub fn new(cwd: PathBuf) -> Result<Self> {
        let board = discover(&cwd)?;
        let links = load_links(&board.links_dir).unwrap_or_default();
        let snapshot = fetch_snapshot_blocking().unwrap_or_default();
        // Issues live on GitHub. If the fetch fails, start with empty boards
        // and mark GitHub stale; the next 30s refresh retries.
        let (github_issues, map_data, stale_github) = match fetch_github_board(&cwd) {
            Ok(github) => (github.issues, github.maps, false),
            Err(_) => (Vec::new(), MapData::new(), true),
        };
        let forest = build_forest(
            board.worktrees.clone(),
            github_issues.clone(),
            links.clone(),
            &snapshot,
        );
        let selected_map = default_map_index(&map_data).unwrap_or(0);

        let mut expanded_branches: HashSet<PathBuf> = HashSet::new();
        for b in &forest.branches {
            expanded_branches.insert(b.worktree.path.clone());
        }
        let mut expanded_issues: HashSet<u64> = HashSet::new();
        for b in &forest.branches {
            if let Some(issue) = &b.issue {
                expanded_issues.insert(issue.issue.number);
            }
        }

        let issues_data = build_issues_data(github_issues.clone(), links, &snapshot);

        let mut app = Self {
            cwd,
            board,
            forest,
            snapshot,
            stale_herdr: false,
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
            map_data,
            selected_map,
            selected_map_child: 0,
            map_detail_scroll: 0,
            stale_github,
            github_issues,
            issues_data,
            selected_issue: 0,
            issues_detail_scroll: 0,
        };
        app.selected_map_child = app.first_map_child();
        app.selected_issue = app.first_issue();
        app.rebuild_flat();
        Ok(app)
    }

    pub fn new_for_test(board: BoardRepository, forest: Forest, snapshot: Snapshot) -> Self {
        let mut expanded_branches: HashSet<PathBuf> = HashSet::new();
        for b in &forest.branches {
            expanded_branches.insert(b.worktree.path.clone());
        }
        let mut expanded_issues: HashSet<u64> = HashSet::new();
        for b in &forest.branches {
            if let Some(issue) = &b.issue {
                expanded_issues.insert(issue.issue.number);
            }
        }
        let mut app = Self {
            cwd: board.primary_path.clone(),
            board,
            forest,
            snapshot,
            stale_herdr: false,
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
            map_data: MapData::new(),
            selected_map: 0,
            selected_map_child: 0,
            map_detail_scroll: 0,
            stale_github: false,
            github_issues: Vec::new(),
            issues_data: IssuesData::default(),
            selected_issue: 0,
            issues_detail_scroll: 0,
        };
        app.selected_map_child = app.first_map_child();
        app.selected_issue = app.first_issue();
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
                if !self.expanded_issues.contains(&issue.issue.number) {
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
        if self.view == View::Map {
            let order = self.map_child_order();
            if let Some(position) = order
                .iter()
                .position(|index| *index == self.selected_map_child)
                && position > 0
            {
                self.selected_map_child = order[position - 1];
                self.map_detail_scroll = 0;
            }
            return;
        }
        if self.view == View::Issues {
            let order = self.issues_order();
            if let Some(position) = order.iter().position(|index| *index == self.selected_issue)
                && position > 0
            {
                self.selected_issue = order[position - 1];
                self.issues_detail_scroll = 0;
            }
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.view == View::Map {
            let order = self.map_child_order();
            if let Some(position) = order
                .iter()
                .position(|index| *index == self.selected_map_child)
                && position + 1 < order.len()
            {
                self.selected_map_child = order[position + 1];
                self.map_detail_scroll = 0;
            }
            return;
        }
        if self.view == View::Issues {
            let order = self.issues_order();
            if let Some(position) = order.iter().position(|index| *index == self.selected_issue)
                && position + 1 < order.len()
            {
                self.selected_issue = order[position + 1];
                self.issues_detail_scroll = 0;
            }
            return;
        }
        if self.selected + 1 < self.flat_nodes.len() {
            self.selected += 1;
        }
    }

    fn map_child_order(&self) -> Vec<usize> {
        let Some(map) = self.map_data.get(self.selected_map) else {
            return Vec::new();
        };
        [
            crate::github::FrontierState::Frontier,
            crate::github::FrontierState::Blocked,
            crate::github::FrontierState::Assigned,
            crate::github::FrontierState::Done,
        ]
        .into_iter()
        .flat_map(|state| {
            map.children
                .iter()
                .enumerate()
                .filter(move |(_, child)| child.state == state)
                .map(|(index, _)| index)
        })
        .collect()
    }

    fn first_map_child(&self) -> usize {
        self.map_child_order().into_iter().next().unwrap_or(0)
    }

    fn issues_order(&self) -> Vec<usize> {
        (0..self.issues_data.len()).collect()
    }

    fn first_issue(&self) -> usize {
        self.issues_order().into_iter().next().unwrap_or(0)
    }

    fn current_issue(&self) -> Option<&crate::issues::IssueRow> {
        self.issues_data.ordered().get(self.selected_issue).copied()
    }

    fn replace_issues_data(&mut self, data: IssuesData) {
        let selected_number = self.current_issue().map(|row| row.issue.number);
        self.issues_data = data;
        self.selected_issue = selected_number
            .and_then(|number| {
                self.issues_data
                    .ordered()
                    .iter()
                    .position(|row| row.issue.number == number)
            })
            .unwrap_or_else(|| self.first_issue());
        self.issues_detail_scroll = 0;
    }

    /// Rebuild Forest and Issues from GitHub issues plus local links.
    fn rebuild_boards(&mut self, issues: Vec<GitHubIssue>, links: Vec<Link>) {
        let forest = build_forest(
            self.board.worktrees.clone(),
            issues.clone(),
            links.clone(),
            &self.snapshot,
        );
        self.forest = forest;
        // Keep expanded sets for existing branches/issues, expand new ones by default
        for b in &self.forest.branches {
            self.expanded_branches.insert(b.worktree.path.clone());
            if let Some(issue) = &b.issue {
                self.expanded_issues.insert(issue.issue.number);
            }
        }
        self.rebuild_flat();
        let issues_data = build_issues_data(issues, links, &self.snapshot);
        self.replace_issues_data(issues_data);
    }

    fn next_map(&mut self) {
        if self.map_data.len() < 2 {
            return;
        }
        self.selected_map = (self.selected_map + 1) % self.map_data.len();
        self.selected_map_child = self.first_map_child();
        self.map_detail_scroll = 0;
    }

    fn current_map_child(&self) -> Option<&crate::github::WayfinderChild> {
        self.map_data
            .get(self.selected_map)
            .and_then(|map| map.children.get(self.selected_map_child))
    }

    fn replace_map_data(&mut self, map_data: MapData) {
        let selected_map_number = self
            .map_data
            .get(self.selected_map)
            .map(|map| map.issue.number);
        let selected_child_number = self.current_map_child().map(|child| child.issue.number);
        self.map_data = map_data;
        self.selected_map = map_index_for_number(&self.map_data, selected_map_number).unwrap_or(0);
        self.selected_map_child = selected_child_number
            .and_then(|number| {
                self.map_data.get(self.selected_map).and_then(|map| {
                    map.children
                        .iter()
                        .position(|child| child.issue.number == number)
                })
            })
            .unwrap_or_else(|| self.first_map_child());
        self.map_detail_scroll = 0;
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
                    // Find issue number
                    let issue_number = if node.branch_idx == usize::MAX {
                        self.forest.unlinked_issues[node.issue_idx.unwrap()]
                            .issue
                            .number
                    } else {
                        self.forest.branches[node.branch_idx]
                            .issue
                            .as_ref()
                            .unwrap()
                            .issue
                            .number
                    };
                    if self.expanded_issues.contains(&issue_number) {
                        self.expanded_issues.remove(&issue_number);
                    } else {
                        self.expanded_issues.insert(issue_number);
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
        if self.view == View::Map && matches!(key.code, KeyCode::Left | KeyCode::Right) {
            return false;
        }
        if self.show_detail && self.view == View::Map {
            match key.code {
                KeyCode::PageUp => {
                    self.map_detail_scroll = self.map_detail_scroll.saturating_sub(5);
                    return false;
                }
                KeyCode::PageDown => {
                    self.map_detail_scroll = self.map_detail_scroll.saturating_add(5);
                    return false;
                }
                KeyCode::Home => {
                    self.map_detail_scroll = 0;
                    return false;
                }
                KeyCode::End => {
                    self.map_detail_scroll = u16::MAX;
                    return false;
                }
                _ => {}
            }
        }
        if self.show_detail && self.view == View::Issues {
            match key.code {
                KeyCode::PageUp => {
                    self.issues_detail_scroll = self.issues_detail_scroll.saturating_sub(5);
                    return false;
                }
                KeyCode::PageDown => {
                    self.issues_detail_scroll = self.issues_detail_scroll.saturating_add(5);
                    return false;
                }
                KeyCode::Home => {
                    self.issues_detail_scroll = 0;
                    return false;
                }
                KeyCode::End => {
                    self.issues_detail_scroll = u16::MAX;
                    return false;
                }
                _ => {}
            }
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
                            let issue_number = if node.branch_idx == usize::MAX {
                                self.forest.unlinked_issues[node.issue_idx.unwrap()]
                                    .issue
                                    .number
                            } else {
                                self.forest.branches[node.branch_idx]
                                    .issue
                                    .as_ref()
                                    .unwrap()
                                    .issue
                                    .number
                            };
                            if self.expanded_issues.contains(&issue_number) {
                                self.expanded_issues.remove(&issue_number);
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
                            let issue_number = if node.branch_idx == usize::MAX {
                                self.forest.unlinked_issues[node.issue_idx.unwrap()]
                                    .issue
                                    .number
                            } else {
                                self.forest.branches[node.branch_idx]
                                    .issue
                                    .as_ref()
                                    .unwrap()
                                    .issue
                                    .number
                            };
                            if !self.expanded_issues.contains(&issue_number) {
                                self.expanded_issues.insert(issue_number);
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
                if self.view == View::Forest
                    || self.current_map_child().is_some()
                    || (self.view == View::Issues && self.current_issue().is_some())
                {
                    self.show_detail = !self.show_detail;
                    self.map_detail_scroll = 0;
                    self.issues_detail_scroll = 0;
                }
            }
            KeyCode::Char('r') => {
                self.status_msg = "refresh requested".to_string();
                // Caller will trigger immediate refresh; we just set flag.
            }
            KeyCode::Char('?') => {
                self.show_help = true;
            }
            KeyCode::Char('1') => {
                self.view = View::Forest;
                self.show_detail = false;
                self.map_detail_scroll = 0;
                self.issues_detail_scroll = 0;
            }
            KeyCode::Char('2') | KeyCode::Char('m') => {
                self.view = View::Map;
                self.show_detail = false;
                self.map_detail_scroll = 0;
                self.issues_detail_scroll = 0;
            }
            KeyCode::Char('3') | KeyCode::Char('i') => {
                self.view = View::Issues;
                self.show_detail = false;
                self.map_detail_scroll = 0;
                self.issues_detail_scroll = 0;
            }
            KeyCode::Tab if self.view == View::Map => self.next_map(),
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
        // Reload worktrees, links, and Herdr snapshot. GitHub issues are cached
        // from the last successful fetch; the 30s GitHub tick refreshes them.
        let links = load_links(&self.board.links_dir).unwrap_or_default();

        // Refresh board discovery for worktrees (in case a new worktree was added).
        if let Ok(new_board) = discover(&self.cwd) {
            self.board = new_board;
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

        let issues = self.github_issues.clone();
        self.rebuild_boards(issues, links);
        self.last_herdr = Instant::now();
    }

    pub fn refresh_github(&mut self) {
        self.last_github = Instant::now();
        match fetch_github_board(&self.cwd) {
            Ok(github) => {
                self.github_issues = github.issues.clone();
                self.replace_map_data(github.maps);
                let links = load_links(&self.board.links_dir).unwrap_or_default();
                self.rebuild_boards(github.issues, links);
                self.stale_github = false;
            }
            Err(_) => {
                self.stale_github = true;
            }
        }
    }

    pub fn needs_herdr_refresh(&self) -> bool {
        self.last_herdr.elapsed() >= Duration::from_secs(2)
    }

    pub fn needs_github_refresh(&self) -> bool {
        self.last_github.elapsed() >= Duration::from_secs(30)
    }
}

// Rendering helpers

pub fn draw_view_tabs(view: View, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let tab_style = |active| {
        if active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        }
    };
    let line = Line::from(vec![
        Span::styled(
            "Views ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("[1 Forest]", tab_style(view == View::Forest)),
        Span::raw(" "),
        Span::styled("[2 Map]", tab_style(view == View::Map)),
        Span::raw(" "),
        Span::styled("[3 Issues]", tab_style(view == View::Issues)),
    ]);
    buf.set_line(area.x + 2, area.y, &line, area.width.saturating_sub(2));
}

pub fn draw_status_bar(app: &App, area: Rect, buf: &mut Buffer) {
    let view_label = match app.view {
        View::Forest => " Forest ",
        View::Map => " Map ",
        View::Issues => " Issues ",
    };
    let mut spans = vec![
        Span::styled(
            view_label.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(
            " q:quit  r:refresh  ?:help  1:Forest  2:Map  3:Issues  Enter:details  ↑/↓:navigate  ←/→:collapse/expand",
        ),
    ];
    if app.stale_herdr {
        spans.push(Span::styled(
            "  ● Herdr stale",
            Style::default().fg(Color::Yellow),
        ));
    }
    if app.stale_github {
        spans.push(Span::styled(
            "  ● GitHub stale",
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
            let title = format!("Issue #{}: {}", issue.number, issue.title);
            let body = format!(
                "State: {}\n\n{}",
                issue.state,
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
    Paragraph::new(body)
        .wrap(Wrap { trim: false })
        .render(inner, buf);
}

pub fn draw_map_detail(app: &App, area: Rect, buf: &mut Buffer) {
    let child = match app.current_map_child() {
        Some(child) => child,
        None => return,
    };

    let mut body = format!("State: {}\n", child.state);
    if !child.issue.assignees.is_empty() {
        body.push_str(&format!(
            "Assignees: {}\n",
            child.issue.assignees.join(", ")
        ));
    }
    if !child.issue.open_blockers.is_empty() {
        body.push_str(&format!(
            "Open blockers: {}\n",
            child
                .issue
                .open_blockers
                .iter()
                .map(|number| format!("#{number}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    body.push('\n');
    body.push_str(&render_markdown_to_text(&child.issue.body));
    for comment in &child.issue.comments {
        body.push_str(&format!("\n\nComment by {}:\n", comment.author));
        body.push_str(&render_markdown_to_text(&comment.body));
    }

    let block = Block::default()
        .title(format!(
            "Issue #{}: {}",
            child.issue.number, child.issue.title
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    block.render(area, buf);
    Clear.render(inner, buf);
    let max_scroll = wrapped_line_count(&body, inner.width)
        .saturating_sub(inner.height as usize)
        .min(u16::MAX as usize) as u16;
    Paragraph::new(body)
        .wrap(Wrap { trim: false })
        .scroll((app.map_detail_scroll.min(max_scroll), 0))
        .render(inner, buf);
}

pub fn draw_issues_detail(app: &App, area: Rect, buf: &mut Buffer) {
    let row = match app.current_issue() {
        Some(row) => row,
        None => return,
    };
    let mut body = format!("State: {}\n", row.state);
    if !row.issue.labels.is_empty() {
        body.push_str(&format!("Labels: {}\n", row.issue.labels.join(", ")));
    }
    if !row.issue.assignees.is_empty() {
        body.push_str(&format!("Assignees: {}\n", row.issue.assignees.join(", ")));
    }
    if row.linked {
        body.push_str("Linked: yes\n");
    }
    if let Some(agent) = &row.agent {
        body.push_str(&format!("Agent: {} {}\n", agent.agent.agent, agent.badge));
    }
    if !row.issue.open_blockers.is_empty() {
        body.push_str(&format!(
            "Open blockers: {}\n",
            row.issue
                .open_blockers
                .iter()
                .map(|number| format!("#{number}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    body.push_str(&format!("\n{}\n", render_markdown_to_text(&row.issue.body)));
    let block = Block::default()
        .title(format!("Issue #{}: {}", row.issue.number, row.issue.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    block.render(area, buf);
    Clear.render(inner, buf);
    let max_scroll = wrapped_line_count(&body, inner.width)
        .saturating_sub(inner.height as usize)
        .min(u16::MAX as usize) as u16;
    Paragraph::new(body)
        .wrap(Wrap { trim: false })
        .scroll((app.issues_detail_scroll.min(max_scroll), 0))
        .render(inner, buf);
}

fn wrapped_line_count(text: &str, width: u16) -> usize {
    if width == 0 {
        return 0;
    }
    text.split('\n')
        .map(|line| {
            let line_width = Line::from(line).width();
            line_width.max(1).div_ceil(width as usize)
        })
        .sum()
}

pub fn draw_help(area: Rect, buf: &mut Buffer) {
    let text = "\
Copse — terminal board

Views:
  1          Forest (branch → issue → agent)
  2, m       Map (Wayfinder frontiers)
  3, i       Issues (every GitHub issue grouped)
  Tab        Next Wayfinder map

Navigation:
  ↑/↓        Move selection
  ←          Collapse or move to parent
  →          Expand or move to child
  Enter      Toggle detail pane
  PgUp/PgDn  Scroll Map Issue detail
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
  Worktrees + links + Herdr every 2s
  GitHub Issues + Wayfinder every 30s
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
                let content_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Min(0)])
                    .split(main_area);
                let view_tabs_area = content_chunks[0];
                let content_area = content_chunks[1];
                draw_view_tabs(app.view, view_tabs_area, f.buffer_mut());

                match app.view {
                    View::Forest => {
                        if app.show_detail {
                            let cols = Layout::default()
                                .direction(Direction::Horizontal)
                                .constraints([
                                    Constraint::Percentage(60),
                                    Constraint::Percentage(40),
                                ])
                                .split(content_area);
                            let forest_area = cols[0];
                            let detail_area = cols[1];
                            ForestWidget {
                                forest: &app.forest,
                                selected: Some(app.selected),
                                expanded_branches: Some(&app.expanded_branches),
                                expanded_issues: Some(&app.expanded_issues),
                            }
                            .render(forest_area, f.buffer_mut());
                            draw_detail(&app, detail_area, f.buffer_mut());
                        } else {
                            ForestWidget {
                                forest: &app.forest,
                                selected: Some(app.selected),
                                expanded_branches: Some(&app.expanded_branches),
                                expanded_issues: Some(&app.expanded_issues),
                            }
                            .render(content_area, f.buffer_mut());
                        }
                    }
                    View::Issues => {
                        if app.show_detail {
                            let cols = Layout::default()
                                .direction(Direction::Horizontal)
                                .constraints([
                                    Constraint::Percentage(60),
                                    Constraint::Percentage(40),
                                ])
                                .split(content_area);
                            let list_area = cols[0];
                            let detail_area = cols[1];
                            IssuesWidget {
                                data: &app.issues_data,
                                selected: Some(app.selected_issue),
                            }
                            .render(list_area, f.buffer_mut());
                            draw_issues_detail(&app, detail_area, f.buffer_mut());
                        } else {
                            IssuesWidget {
                                data: &app.issues_data,
                                selected: Some(app.selected_issue),
                            }
                            .render(content_area, f.buffer_mut());
                        }
                    }
                    View::Map => {
                        if app.show_detail {
                            let cols = Layout::default()
                                .direction(Direction::Horizontal)
                                .constraints([
                                    Constraint::Percentage(68),
                                    Constraint::Percentage(32),
                                ])
                                .split(content_area);
                            let map_area = cols[0];
                            let detail_area = cols[1];
                            MapWidget {
                                maps: &app.map_data,
                                selected_map: app.selected_map,
                                selected_child: app
                                    .current_map_child()
                                    .map(|_| app.selected_map_child),
                            }
                            .render(map_area, f.buffer_mut());
                            draw_map_detail(&app, detail_area, f.buffer_mut());
                        } else {
                            MapWidget {
                                maps: &app.map_data,
                                selected_map: app.selected_map,
                                selected_child: app
                                    .current_map_child()
                                    .map(|_| app.selected_map_child),
                            }
                            .render(content_area, f.buffer_mut());
                        }
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
                                    app.refresh_github();
                                }
                            }
                            Event::Mouse(me) => {
                                match me.kind {
                                    MouseEventKind::Down(MouseButton::Left) => {
                                        if app.view == View::Forest {
                                            let y = me.row as usize;
                                            if y < app.flat_nodes.len() {
                                                app.selected = y;
                                            }
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
                    app.refresh_github();
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
    use crate::github::{
        FrontierState, GitHubComment, GitHubIssue, IssueState, WayfinderChild, WayfinderMap,
    };
    use crate::herdr::{AgentStatus, Snapshot};
    use crate::tracker::Link;
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
            links_dir: PathBuf::from("/tmp/repo/.copse/links"),
            is_copse_present: false,
        }
    }

    fn sample_issue(number: u64, title: &str) -> GitHubIssue {
        GitHubIssue {
            number,
            title: title.to_string(),
            state: IssueState::Open,
            body: "body".to_string(),
            comments: Vec::new(),
            labels: Vec::new(),
            assignees: Vec::new(),
            open_blockers: Vec::new(),
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
        let issue = sample_issue(7, "Issue");
        let link = Link {
            id: Uuid::new_v4(),
            issue: issue.number,
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

    fn test_map(number: u64, title: &str) -> WayfinderMap {
        WayfinderMap {
            issue: GitHubIssue {
                number,
                title: title.to_string(),
                state: IssueState::Closed,
                body: String::new(),
                comments: Vec::new(),
                labels: vec!["wayfinder:map".to_string()],
                assignees: Vec::new(),
                open_blockers: Vec::new(),
            },
            children: vec![
                WayfinderChild {
                    issue: GitHubIssue {
                        number: number + 1,
                        title: "Done".to_string(),
                        state: IssueState::Closed,
                        body: String::new(),
                        comments: Vec::new(),
                        labels: Vec::new(),
                        assignees: Vec::new(),
                        open_blockers: Vec::new(),
                    },
                    state: FrontierState::Done,
                },
                WayfinderChild {
                    issue: GitHubIssue {
                        number: number + 2,
                        title: "Frontier".to_string(),
                        state: IssueState::Open,
                        body: String::new(),
                        comments: Vec::new(),
                        labels: Vec::new(),
                        assignees: Vec::new(),
                        open_blockers: Vec::new(),
                    },
                    state: FrontierState::Frontier,
                },
            ],
            open_child_count: 1,
        }
    }

    #[test]
    fn map_view_selects_frontier_and_cycles_maps() {
        let board = repo_with_worktrees();
        let mut app = App::new_for_test(board, Forest::default(), Snapshot::default());
        app.map_data = vec![test_map(10, "First"), test_map(20, "Second")];
        app.selected_map = 0;
        app.selected_map_child = app.first_map_child();
        app.view = View::Map;

        assert_eq!(app.selected_map_child, 1);
        app.move_down();
        assert_eq!(app.selected_map_child, 0);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.selected_map, 1);
        assert_eq!(app.selected_map_child, 1);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.selected_map, 0);
    }

    #[test]
    fn view_tabs_show_both_views_and_highlight_the_current_one() {
        let area = Rect::new(0, 0, 50, 1);
        let mut buf = Buffer::empty(area);
        draw_view_tabs(View::Map, area, &mut buf);
        let rendered: String = (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();

        assert!(rendered.contains("Views [1 Forest] [2 Map]"));
        assert!((0..area.width).any(|x| buf[(x, 0)].bg == Color::Cyan));
    }

    #[test]
    fn map_detail_wraps_body_and_includes_comments() {
        let board = repo_with_worktrees();
        let mut map = test_map(10, "Map");
        map.children[0].issue.body = "## Question\n\nThis is a long question with a wrap-marker that should continue on another line in the detail pane.\n".to_string();
        map.children[0].issue.comments = vec![GitHubComment {
            author: "zac".to_string(),
            body: "## Answer\n\nThis is the actual answer content.".to_string(),
        }];
        let mut app = App::new_for_test(board, Forest::default(), Snapshot::default());
        app.map_data = vec![map];
        app.selected_map = 0;
        app.selected_map_child = 0;

        let area = Rect::new(0, 0, 32, 24);
        let mut buf = Buffer::empty(area);
        draw_map_detail(&app, area, &mut buf);
        let rendered: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("wrap-marker"));
        assert!(rendered.contains("actual answer"));
        assert!(rendered.contains("content."));
    }

    #[test]
    fn forest_detail_wraps_long_body() {
        let board = repo_with_worktrees();
        let mut issue = sample_issue(9, "Long Issue");
        issue.body = "This is a very long body with a wrap-marker-forest that should wrap across multiple lines in the detail pane even though it is a single paragraph without newlines. ".repeat(2);
        let wt = Worktree {
            path: PathBuf::from("/tmp/repo"),
            head: "abc".to_string(),
            branch: Some("refs/heads/main".to_string()),
            is_bare: false,
            is_detached: false,
            is_prunable: false,
        };
        let link = Link {
            id: Uuid::new_v4(),
            issue: issue.number,
            worktree: "/tmp/repo".to_string(),
            body: "".to_string(),
            extra: HashMap::new(),
        };
        let forest = build_forest(vec![wt], vec![issue], vec![link], &Snapshot::default());
        let mut app = App::new_for_test(board, forest, Snapshot::default());
        // flat: 0 branch, 1 issue, 2 agent placeholder - select issue
        app.selected = 1;
        let area = Rect::new(0, 0, 32, 12);
        let mut buf = Buffer::empty(area);
        draw_detail(&app, area, &mut buf);
        let rendered: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("wrap-marker-forest"),
            "long body should be rendered with wrapping: {rendered}"
        );
        // Ensure it wrapped - the marker should not be truncated and body should use multiple lines
        assert!(rendered.lines().count() > 3);
    }

    #[test]
    fn map_detail_scroll_keys_move_through_issue_content() {
        let board = repo_with_worktrees();
        let mut app = App::new_for_test(board, Forest::default(), Snapshot::default());
        app.map_data = vec![test_map(10, "Map")];
        app.view = View::Map;
        app.show_detail = true;

        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.map_detail_scroll, 5);
        app.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        assert_eq!(app.map_detail_scroll, 0);
        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(app.map_detail_scroll, u16::MAX);
        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(app.map_detail_scroll, 0);
    }

    #[test]
    fn github_refresh_keeps_last_good_data_on_failure() {
        let board = repo_with_worktrees();
        let mut app = App::new_for_test(board, Forest::default(), Snapshot::default());
        app.map_data = vec![test_map(10, "Existing")];
        app.selected_map = 0;
        app.cwd = PathBuf::from("/definitely-not-a-copse-repository");

        app.refresh_github();

        assert!(app.stale_github);
        assert_eq!(app.map_data[0].issue.title, "Existing");
    }

    #[test]
    fn refresh_rebuilds_from_cached_github_issues() {
        // Refresh must not panic when .copse links and Herdr are missing, and
        // must rebuild the forest from cached GitHub issues plus worktrees.
        let board = repo_with_worktrees();
        let forest = Forest::default();
        let snap = Snapshot::default();
        let mut app = App::new_for_test(board, forest, snap);
        app.github_issues = vec![sample_issue(7, "Cached")];
        app.refresh();
        // Two worktrees from the board, none linked (no link files on disk).
        assert_eq!(app.forest.branches.len(), 2);
        assert!(app.forest.branches.iter().all(|b| b.issue.is_none()));
        // The cached issue shows up as unlinked: open, unblocked, no map label.
        assert_eq!(app.forest.unlinked_issues.len(), 1);
        assert_eq!(app.issues_data.len(), 1);
    }
}
