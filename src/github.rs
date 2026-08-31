#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use crate::tracker::{Issue, Status, load_issues, load_links};

use serde::Deserialize;
use thiserror::Error;

const ISSUE_FIELDS: &str =
    "number,title,state,body,labels,assignees,comments,blockedBy,parent,subIssues";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
    Open,
    Closed,
}

impl IssueState {
    fn parse(value: &str) -> Result<Self, GitHubError> {
        match value.to_ascii_lowercase().as_str() {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            other => Err(GitHubError::InvalidState(other.to_string())),
        }
    }
}

impl std::fmt::Display for IssueState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubComment {
    pub author: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub state: IssueState,
    pub body: String,
    pub comments: Vec<GitHubComment>,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub open_blockers: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontierState {
    Frontier,
    Blocked,
    Assigned,
    Done,
}

impl std::fmt::Display for FrontierState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Frontier => write!(f, "frontier"),
            Self::Blocked => write!(f, "blocked"),
            Self::Assigned => write!(f, "assigned"),
            Self::Done => write!(f, "done"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WayfinderChild {
    pub issue: GitHubIssue,
    pub state: FrontierState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WayfinderMap {
    pub issue: GitHubIssue,
    pub children: Vec<WayfinderChild>,
    pub open_child_count: usize,
}

pub type MapData = Vec<WayfinderMap>;

#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("gh not found: {0}")]
    NotFound(String),
    #[error("gh command failed: {0}")]
    CommandFailed(String),
    #[error("failed to parse GitHub issues: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("unknown GitHub issue state: {0}")]
    InvalidState(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("local tracker error: {0}")]
    Local(String),
}

#[derive(Debug, Deserialize)]
struct RawIssue {
    number: u64,
    title: String,
    state: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    labels: Vec<RawLabel>,
    #[serde(default)]
    assignees: Vec<RawAssignee>,
    #[serde(default)]
    comments: Vec<RawComment>,
    #[serde(default, rename = "blockedBy")]
    blocked_by: Option<RawConnection>,
    #[serde(default)]
    parent: Option<RawReference>,
    #[serde(default, rename = "subIssues")]
    sub_issues: Option<RawConnection>,
}

#[derive(Debug, Deserialize)]
struct RawLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawAssignee {
    login: String,
}

#[derive(Debug, Deserialize)]
struct RawComment {
    #[serde(default)]
    author: Option<RawAssignee>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawConnection {
    #[serde(default)]
    nodes: Vec<RawReference>,
}

#[derive(Debug, Deserialize)]
struct RawReference {
    number: u64,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Clone)]
struct IssueRecord {
    issue: GitHubIssue,
    native_blockers: Vec<RawBlocker>,
    parent: Option<u64>,
    sub_issues: Vec<u64>,
}

#[derive(Debug, Clone)]
struct RawBlocker {
    number: u64,
    state: Option<String>,
}

pub fn classify_issue(issue: &GitHubIssue) -> FrontierState {
    match issue.state {
        IssueState::Closed => FrontierState::Done,
        IssueState::Open if !issue.open_blockers.is_empty() => FrontierState::Blocked,
        IssueState::Open if !issue.assignees.is_empty() => FrontierState::Assigned,
        IssueState::Open => FrontierState::Frontier,
    }
}

pub fn default_map_index(maps: &[WayfinderMap]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (index, map) in maps.iter().enumerate() {
        let replace = match best {
            None => true,
            Some(best_index) => {
                let current = &maps[best_index];
                map.open_child_count > current.open_child_count
                    || (map.open_child_count == current.open_child_count
                        && map.issue.number < current.issue.number)
            }
        };
        if replace {
            best = Some(index);
        }
    }
    best
}

pub fn map_index_for_number(maps: &[WayfinderMap], selected_number: Option<u64>) -> Option<usize> {
    selected_number
        .and_then(|number| maps.iter().position(|map| map.issue.number == number))
        .or_else(|| default_map_index(maps))
}

pub fn fetch_wayfinder_maps(
    cwd: &Path,
    local_issues_dir: &Path,
) -> Result<(MapData, bool), GitHubError> {
    let local_maps = fetch_local_wayfinder_maps(local_issues_dir).unwrap_or_default();

    let github_maps = fetch_github_wayfinder_maps(cwd);
    match github_maps {
        Ok(mut maps) => {
            let mut combined = local_maps;
            combined.append(&mut maps);
            Ok((combined, false))
        }
        Err(_error) if !local_maps.is_empty() => Ok((local_maps, true)),
        Err(error) => Err(error),
    }
}

fn fetch_github_wayfinder_maps(cwd: &Path) -> Result<MapData, GitHubError> {
    let output = Command::new("gh")
        .current_dir(cwd)
        .args([
            "issue",
            "list",
            "--state",
            "all",
            "--limit",
            "1000",
            "--json",
            ISSUE_FIELDS,
        ])
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                GitHubError::NotFound(error.to_string())
            } else {
                GitHubError::Io(error)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(GitHubError::CommandFailed(stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_wayfinder_maps(&stdout)
}

pub fn fetch_local_wayfinder_maps(issues_dir: &Path) -> Result<MapData, GitHubError> {
    let mut issues =
        load_issues(issues_dir).map_err(|error| GitHubError::Local(error.to_string()))?;
    issues.sort_by_key(|issue| issue.id);

    let numbers = issues
        .iter()
        .enumerate()
        .map(|(index, issue)| (issue.id, index as u64 + 1))
        .collect::<HashMap<_, _>>();
    let by_id = issues
        .iter()
        .map(|issue| (issue.id, issue))
        .collect::<HashMap<_, _>>();

    // Links indicate a claim. For Copse, a linked open issue is Assigned (the
    // local equivalent of GitHub assignees). Infer links_dir as sibling of
    // issues_dir (.copse/issues -> .copse/links) which covers App's board layout
    // and the new test layout (tmp/issues + tmp/links).
    let linked_ids: HashSet<uuid::Uuid> = {
        let mut set = HashSet::new();
        if let Some(parent) = issues_dir.parent() {
            let candidate = parent.join("links");
            if let Ok(links) = load_links(&candidate) {
                for link in links {
                    set.insert(link.issue);
                }
            }
        }
        // Fallback: if issues_dir itself is the temp dir that directly contains
        // link files (older unit test layout), also check there.
        if set.is_empty() {
            if let Ok(links) = load_links(issues_dir) {
                for link in links {
                    // Only treat as linked if the file actually parses as a link;
                    // issue files in same dir will be ignored by load_links (parse fails) and are skipped.
                    set.insert(link.issue);
                }
            }
        }
        set
    };

    let mut maps = issues
        .iter()
        .filter(|issue| {
            issue_labels(issue)
                .iter()
                .any(|label| label == "wayfinder:map")
        })
        .map(|map| {
            let children = issues
                .iter()
                .filter(|issue| parse_parent_id(&issue.body) == Some(map.id))
                .map(|child| local_child(child, &numbers, &by_id, &linked_ids))
                .collect::<Vec<_>>();
            WayfinderMap {
                issue: local_issue_with_assignees(map, numbers[&map.id], Vec::new(), false),
                open_child_count: children
                    .iter()
                    .filter(|child| child.issue.state == IssueState::Open)
                    .count(),
                children,
            }
        })
        .collect::<Vec<_>>();
    maps.sort_by_key(|map| map.issue.number);
    Ok(maps)
}

fn local_child(
    issue: &Issue,
    numbers: &HashMap<uuid::Uuid, u64>,
    by_id: &HashMap<uuid::Uuid, &Issue>,
    linked_ids: &HashSet<uuid::Uuid>,
) -> WayfinderChild {
    let blockers = parse_uuid_references(&issue.body)
        .into_iter()
        .filter(|id| {
            by_id
                .get(id)
                .map(|blocker| blocker.status != Status::Closed)
                .unwrap_or(true)
        })
        .filter_map(|id| numbers.get(&id).copied())
        .collect::<Vec<_>>();
    let is_linked = linked_ids.contains(&issue.id);
    let github_issue = local_issue_with_assignees(issue, numbers[&issue.id], blockers, is_linked);
    let state = classify_issue(&github_issue);
    WayfinderChild {
        issue: github_issue,
        state,
    }
}

fn local_issue(issue: &Issue, number: u64, open_blockers: Vec<u64>) -> GitHubIssue {
    local_issue_with_assignees(issue, number, open_blockers, false)
}

fn local_issue_with_assignees(
    issue: &Issue,
    number: u64,
    open_blockers: Vec<u64>,
    is_linked: bool,
) -> GitHubIssue {
    GitHubIssue {
        number,
        title: issue.title.clone(),
        state: match issue.status {
            Status::Open => IssueState::Open,
            Status::Closed | Status::Archived => IssueState::Closed,
        },
        body: issue.body.clone(),
        comments: Vec::new(),
        labels: issue_labels(issue),
        // For local Copse, a link is the claim signal, equivalent to GitHub assignee.
        assignees: if is_linked {
            vec!["linked".to_string()]
        } else {
            Vec::new()
        },
        open_blockers,
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

fn parse_parent_id(body: &str) -> Option<uuid::Uuid> {
    body.lines().take(12).find_map(|line| {
        let lower = line.to_ascii_lowercase();
        let value = lower
            .find("parent:")
            .map(|index| &line[index + "parent:".len()..])?;
        value.split_whitespace().find_map(|part| {
            uuid::Uuid::parse_str(
                part.trim_matches(|ch: char| !ch.is_ascii_hexdigit() && ch != '-'),
            )
            .ok()
        })
    })
}

fn parse_uuid_references(body: &str) -> Vec<uuid::Uuid> {
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
            uuid::Uuid::parse_str(
                part.trim_matches(|ch: char| !ch.is_ascii_hexdigit() && ch != '-'),
            )
            .ok()
        })
        .collect()
}

pub fn parse_wayfinder_maps(json: &str) -> Result<MapData, GitHubError> {
    let raw_issues: Vec<RawIssue> = serde_json::from_str(json)?;
    let records = raw_issues
        .into_iter()
        .map(IssueRecord::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let by_number: HashMap<u64, IssueRecord> = records
        .iter()
        .cloned()
        .map(|record| (record.issue.number, record))
        .collect();

    let mut maps = Vec::new();
    for record in records.iter().filter(|record| {
        record
            .issue
            .labels
            .iter()
            .any(|label| label == "wayfinder:map")
    }) {
        let child_numbers = child_numbers(record, &records);
        let children = child_numbers
            .into_iter()
            .filter_map(|number| by_number.get(&number))
            .map(|child| {
                let mut issue = child.issue.clone();
                issue.open_blockers = resolve_open_blockers(child, &by_number);
                let state = classify_issue(&issue);
                WayfinderChild { issue, state }
            })
            .collect::<Vec<_>>();

        maps.push(WayfinderMap {
            issue: record.issue.clone(),
            open_child_count: children
                .iter()
                .filter(|child| child.issue.state == IssueState::Open)
                .count(),
            children,
        });
    }

    maps.sort_by_key(|map| map.issue.number);
    Ok(maps)
}

impl TryFrom<RawIssue> for IssueRecord {
    type Error = GitHubError;

    fn try_from(raw: RawIssue) -> Result<Self, Self::Error> {
        let state = IssueState::parse(&raw.state)?;
        let labels = raw.labels.into_iter().map(|label| label.name).collect();
        let assignees = raw
            .assignees
            .into_iter()
            .map(|assignee| assignee.login)
            .collect();
        let comments = raw
            .comments
            .into_iter()
            .map(|comment| GitHubComment {
                author: comment
                    .author
                    .map(|author| author.login)
                    .unwrap_or_else(|| "unknown".to_string()),
                body: comment.body.unwrap_or_default(),
            })
            .collect();
        let native_blockers = raw
            .blocked_by
            .unwrap_or(RawConnection { nodes: Vec::new() })
            .nodes
            .into_iter()
            .map(|reference| RawBlocker {
                number: reference.number,
                state: reference.state,
            })
            .collect();
        let parent = raw.parent.map(|reference| reference.number);
        let sub_issues = raw
            .sub_issues
            .unwrap_or(RawConnection { nodes: Vec::new() })
            .nodes
            .into_iter()
            .map(|reference| reference.number)
            .collect();

        Ok(Self {
            issue: GitHubIssue {
                number: raw.number,
                title: raw.title,
                state,
                body: raw.body.unwrap_or_default(),
                comments,
                labels,
                assignees,
                open_blockers: Vec::new(),
            },
            native_blockers,
            parent,
            sub_issues,
        })
    }
}

fn child_numbers(map: &IssueRecord, records: &[IssueRecord]) -> Vec<u64> {
    if !map.sub_issues.is_empty() {
        return dedupe(map.sub_issues.clone());
    }

    let task_list = parse_task_list_numbers(&map.issue.body);
    if !task_list.is_empty() {
        return dedupe(task_list);
    }

    let mut fallback = records
        .iter()
        .filter(|record| {
            record.parent == Some(map.issue.number)
                || has_part_of_marker(&record.issue.body, map.issue.number)
        })
        .map(|record| record.issue.number)
        .collect::<Vec<_>>();
    fallback.sort_unstable();
    dedupe(fallback)
}

fn resolve_open_blockers(record: &IssueRecord, by_number: &HashMap<u64, IssueRecord>) -> Vec<u64> {
    let mut blockers = HashSet::new();

    for blocker in &record.native_blockers {
        if blocker.state.as_deref().map(is_open_state).unwrap_or(true) {
            blockers.insert(blocker.number);
        }
    }

    for number in parse_blocked_by_numbers(&record.issue.body) {
        match by_number.get(&number) {
            Some(blocker) if blocker.issue.state == IssueState::Closed => {}
            Some(_) | None => {
                blockers.insert(number);
            }
        }
    }

    let mut result = blockers.into_iter().collect::<Vec<_>>();
    result.sort_unstable();
    result
}

fn is_open_state(state: &str) -> bool {
    state.eq_ignore_ascii_case("open")
}

fn parse_task_list_numbers(body: &str) -> Vec<u64> {
    let mut numbers = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        let bytes = trimmed.as_bytes();
        if bytes.len() < 5
            || !matches!(bytes[0], b'-' | b'*')
            || bytes[1] != b' '
            || bytes[2] != b'['
            || !matches!(bytes[3], b' ' | b'x' | b'X')
            || bytes[4] != b']'
        {
            continue;
        }
        numbers.extend(parse_issue_references(&trimmed[5..]));
    }
    numbers
}

fn parse_blocked_by_numbers(body: &str) -> Vec<u64> {
    let mut numbers = Vec::new();
    for line in body.lines().take(12) {
        let lower = line.to_ascii_lowercase();
        if let Some(index) = lower.find("blocked by:") {
            numbers.extend(parse_issue_references(&line[index + "blocked by:".len()..]));
        }
    }
    dedupe(numbers)
}

fn parse_issue_references(text: &str) -> Vec<u64> {
    let bytes = text.as_bytes();
    let mut numbers = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let start = if bytes[index] == b'#' {
            Some(index + 1)
        } else if bytes[index..].starts_with(b"/issues/") {
            Some(index + b"/issues/".len())
        } else {
            None
        };
        if let Some(start) = start {
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start
                && let Ok(number) = text[start..end].parse::<u64>()
            {
                numbers.push(number);
                index = end;
                continue;
            }
        }
        index += 1;
    }
    numbers
}

fn has_part_of_marker(body: &str, map_number: u64) -> bool {
    let expected = format!("part of #{map_number}");
    body.lines()
        .take(6)
        .any(|line| line.to_ascii_lowercase().contains(&expected))
}

fn dedupe(numbers: Vec<u64>) -> Vec<u64> {
    let mut seen = HashSet::new();
    numbers
        .into_iter()
        .filter(|number| seen.insert(*number))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(
        number: u64,
        title: &str,
        state: IssueState,
        assignees: &[&str],
        open_blockers: &[u64],
    ) -> GitHubIssue {
        GitHubIssue {
            number,
            title: title.to_string(),
            state,
            body: String::new(),
            comments: Vec::new(),
            labels: Vec::new(),
            assignees: assignees.iter().map(|name| (*name).to_string()).collect(),
            open_blockers: open_blockers.to_vec(),
        }
    }

    #[test]
    fn frontier_state_uses_blockers_before_assignees() {
        assert_eq!(
            classify_issue(&issue(1, "ready", IssueState::Open, &[], &[])),
            FrontierState::Frontier
        );
        assert_eq!(
            classify_issue(&issue(2, "blocked", IssueState::Open, &[], &[9])),
            FrontierState::Blocked
        );
        assert_eq!(
            classify_issue(&issue(3, "claimed", IssueState::Open, &["zac"], &[])),
            FrontierState::Assigned
        );
        assert_eq!(
            classify_issue(&issue(4, "closed", IssueState::Closed, &["zac"], &[9])),
            FrontierState::Done
        );
        assert_eq!(
            classify_issue(&issue(
                5,
                "blocked and claimed",
                IssueState::Open,
                &["zac"],
                &[9]
            )),
            FrontierState::Blocked
        );
    }

    #[test]
    fn parses_sub_issue_order_and_uppercase_states() {
        let json = r#"
        [
          {
            "number": 1,
            "title": "Copse",
            "state": "CLOSED",
            "body": "",
            "labels": [{"name":"wayfinder:map"}],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[{"number":3},{"number":2}],"totalCount":2}
          },
          {
            "number": 2,
            "title": "Second",
            "state": "OPEN",
            "body": "",
            "labels": [{"name":"wayfinder:task"}],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": {"number":1},
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 3,
            "title": "First",
            "state": "CLOSED",
            "body": "",
            "labels": [{"name":"wayfinder:research"}],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": {"number":1},
            "subIssues": {"nodes":[],"totalCount":0}
          }
        ]
        "#;

        let maps = parse_wayfinder_maps(json).unwrap();
        assert_eq!(maps.len(), 1);
        assert_eq!(
            maps[0]
                .children
                .iter()
                .map(|child| child.issue.number)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert_eq!(maps[0].children[0].state, FrontierState::Done);
        assert_eq!(maps[0].children[1].state, FrontierState::Frontier);
    }

    #[test]
    fn keeps_wayfinder_comments_with_the_issue() {
        let json = r###"
        [
          {
            "number": 1,
            "title": "Map",
            "state": "OPEN",
            "body": "- [ ] #2",
            "labels": [{"name":"wayfinder:map"}],
            "assignees": [],
            "comments": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 2,
            "title": "Question",
            "state": "CLOSED",
            "body": "## Question\n\nWhat should we do?\n",
            "labels": [],
            "assignees": [],
            "comments": [{
              "author": {"login":"zacd1302-ops"},
              "body": "## Answer\n\nUse the Forest layout."
            }],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          }
        ]
        "###;

        let maps = parse_wayfinder_maps(json).unwrap();
        assert_eq!(maps[0].children[0].issue.comments.len(), 1);
        assert_eq!(
            maps[0].children[0].issue.comments[0].body,
            "## Answer\n\nUse the Forest layout."
        );
    }

    #[test]
    fn parses_task_list_fallback_and_ignores_checkbox_state() {
        let json = r#"
        [
          {
            "number": 10,
            "title": "Build",
            "state": "CLOSED",
            "body": "- [x] #12\n- [ ] #11\n",
            "labels": [{"name":"wayfinder:map"}],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 11,
            "title": "Open issue",
            "state": "OPEN",
            "body": "",
            "labels": [],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 12,
            "title": "Closed issue",
            "state": "CLOSED",
            "body": "",
            "labels": [],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          }
        ]
        "#;

        let maps = parse_wayfinder_maps(json).unwrap();
        assert_eq!(
            maps[0]
                .children
                .iter()
                .map(|child| child.issue.number)
                .collect::<Vec<_>>(),
            vec![12, 11]
        );
        assert_eq!(maps[0].open_child_count, 1);
    }

    #[test]
    fn resolves_native_and_fallback_blockers_conservatively() {
        let json = r#"
        [
          {
            "number": 1,
            "title": "Map",
            "state": "OPEN",
            "body": "- [ ] #4",
            "labels": [{"name":"wayfinder:map"}],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 4,
            "title": "Child",
            "state": "OPEN",
            "body": "Blocked by: #2, [#3](https://github.com/x/y/issues/3), #99.\nDetails",
            "labels": [],
            "assignees": [],
            "blockedBy": {"nodes":[
              {"number":5,"state":"OPEN"},
              {"number":6,"state":"CLOSED"}
            ],"totalCount":2},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 2,
            "title": "Open blocker",
            "state": "OPEN",
            "body": "",
            "labels": [],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 3,
            "title": "Closed blocker",
            "state": "CLOSED",
            "body": "",
            "labels": [],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 5,
            "title": "Native open blocker",
            "state": "OPEN",
            "body": "",
            "labels": [],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 6,
            "title": "Native closed blocker",
            "state": "CLOSED",
            "body": "",
            "labels": [],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          }
        ]
        "#;

        let maps = parse_wayfinder_maps(json).unwrap();
        assert_eq!(maps[0].children[0].issue.open_blockers, vec![2, 5, 99]);
        assert_eq!(maps[0].children[0].state, FrontierState::Blocked);
    }

    #[test]
    fn parses_references_after_unicode_text() {
        assert_eq!(
            parse_issue_references("→ résumé /issues/12 and #13"),
            vec![12, 13]
        );
    }

    #[test]
    fn part_of_fallback_orders_by_issue_number() {
        let json = r#"
        [
          {
            "number": 10,
            "title": "Map",
            "state": "OPEN",
            "body": "No task list",
            "labels": [{"name":"wayfinder:map"}],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 12,
            "title": "Twelve",
            "state": "OPEN",
            "body": "Part of #10\n",
            "labels": [],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          },
          {
            "number": 11,
            "title": "Eleven",
            "state": "CLOSED",
            "body": "Part of #10\n",
            "labels": [],
            "assignees": [],
            "blockedBy": {"nodes":[],"totalCount":0},
            "parent": null,
            "subIssues": {"nodes":[],"totalCount":0}
          }
        ]
        "#;

        let maps = parse_wayfinder_maps(json).unwrap();
        assert_eq!(
            maps[0]
                .children
                .iter()
                .map(|child| child.issue.number)
                .collect::<Vec<_>>(),
            vec![11, 12]
        );
    }

    #[test]
    fn loads_local_maps_and_uuid_relationships() {
        let dir = tempfile::tempdir().unwrap();
        let map_id = uuid::Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
        let blocker_id = uuid::Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();
        let child_id = uuid::Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap();
        let labels = |values: &[&str]| {
            toml::Value::Array(
                values
                    .iter()
                    .map(|value| toml::Value::String((*value).to_string()))
                    .collect(),
            )
        };
        for issue in [
            Issue {
                id: map_id,
                title: "Local map".to_string(),
                status: Status::Open,
                body: String::new(),
                extra: HashMap::from([(String::from("labels"), labels(&["wayfinder:map"]))]),
            },
            Issue {
                id: blocker_id,
                title: "Blocker".to_string(),
                status: Status::Open,
                body: String::new(),
                extra: HashMap::new(),
            },
            Issue {
                id: child_id,
                title: "Child".to_string(),
                status: Status::Open,
                body: format!("Parent: {map_id}\nBlocked by: {blocker_id}\n"),
                extra: HashMap::new(),
            },
        ] {
            crate::tracker::write_issue_file(&issue, dir.path()).unwrap();
        }

        let maps = fetch_local_wayfinder_maps(dir.path()).unwrap();
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].issue.title, "Local map");
        assert_eq!(maps[0].children.len(), 1);
        assert_eq!(maps[0].children[0].issue.title, "Child");
        assert_eq!(maps[0].children[0].issue.open_blockers, vec![2]);
        assert_eq!(maps[0].children[0].state, FrontierState::Blocked);
    }

    #[test]
    fn local_map_linked_issue_is_assigned() {
        // Repro for claim label bug: linked open child should be Assigned, not Frontier.
        // Currently fetch_local_wayfinder_maps ignores .copse/links, so linked child stays Frontier.
        let dir = tempfile::tempdir().unwrap();
        let issues_dir = dir.path().join("issues");
        let links_dir = dir.path().join("links");
        std::fs::create_dir_all(&issues_dir).unwrap();
        std::fs::create_dir_all(&links_dir).unwrap();
        let map_id = uuid::Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
        let child_open_id = uuid::Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap();
        let child_linked_id =
            uuid::Uuid::parse_str("44444444-4444-4444-8444-444444444444").unwrap();
        let labels = |values: &[&str]| {
            toml::Value::Array(
                values
                    .iter()
                    .map(|value| toml::Value::String((*value).to_string()))
                    .collect(),
            )
        };
        for issue in [
            Issue {
                id: map_id,
                title: "Local map".to_string(),
                status: Status::Open,
                body: String::new(),
                extra: HashMap::from([(String::from("labels"), labels(&["wayfinder:map"]))]),
            },
            Issue {
                id: child_open_id,
                title: "Frontier child".to_string(),
                status: Status::Open,
                body: format!("Parent: {map_id}\n"),
                extra: HashMap::new(),
            },
            Issue {
                id: child_linked_id,
                title: "Linked child".to_string(),
                status: Status::Open,
                body: format!("Parent: {map_id}\n"),
                extra: HashMap::new(),
            },
        ] {
            crate::tracker::write_issue_file(&issue, &issues_dir).unwrap();
        }
        // Link the second child to main worktree, simulating a claim still on main
        let link = crate::tracker::Link {
            id: uuid::Uuid::new_v4(),
            issue: child_linked_id,
            worktree: "/tmp/repo".to_string(),
            body: String::new(),
            extra: HashMap::new(),
        };
        crate::tracker::write_link_file(&link, &links_dir).unwrap();

        // Before fix: both children Frontier. After fix: linked one Assigned.
        // Use fetch_local_wayfinder_maps_with_links if available, else fetch_local_wayfinder_maps should consider links.
        let maps = fetch_local_wayfinder_maps(&issues_dir).unwrap();
        // This will be red until fetch_local_wayfinder_maps checks links. Force the assertion to expose bug:
        // We expect 2 children, one Frontier one Assigned, but current code gives 2 Frontier.
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].children.len(), 2);
        let mut states = maps[0]
            .children
            .iter()
            .map(|c| (c.issue.title.clone(), c.state))
            .collect::<Vec<_>>();
        states.sort_by(|a, b| a.0.cmp(&b.0));
        // Frontier child should stay Frontier, linked should be Assigned
        assert_eq!(states[0].1, FrontierState::Frontier, "Frontier child");
        assert_eq!(
            states[1].1,
            FrontierState::Assigned,
            "Linked child should be Assigned when link exists"
        );
    }

    #[test]
    fn default_map_prefers_open_children_then_lowest_number() {
        let make_map = |number, open_child_count| WayfinderMap {
            issue: issue(number, "map", IssueState::Closed, &[], &[]),
            children: Vec::new(),
            open_child_count,
        };
        let maps = vec![make_map(10, 2), make_map(1, 2), make_map(2, 1)];
        assert_eq!(default_map_index(&maps), Some(1));
        assert_eq!(map_index_for_number(&maps, Some(10)), Some(0));
        assert_eq!(map_index_for_number(&maps, Some(99)), Some(1));
    }
}
