#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Open,
    Closed,
    Archived,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Open => write!(f, "open"),
            Status::Closed => write!(f, "closed"),
            Status::Archived => write!(f, "archived"),
        }
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("missing front matter delimiter")]
    MissingFrontMatter,
    #[error("invalid TOML: {0}")]
    InvalidToml(#[from] toml::de::Error),
    #[error("invalid TOML serialization: {0}")]
    InvalidTomlSer(#[from] toml::ser::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid status: {0}")]
    InvalidStatus(String),
    #[error("invalid UUID for {field}: {source}")]
    InvalidUuid {
        field: &'static str,
        source: uuid::Error,
    },
    #[error("filename id mismatch: file stem {file_stem} does not match id {id}")]
    IdMismatch { file_stem: String, id: String },
    #[error("refusing to overwrite malformed record at {path}: {reason}")]
    RefusingOverwrite { path: String, reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub id: Uuid,
    pub title: String,
    pub status: Status,
    pub body: String,
    pub extra: HashMap<String, toml::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub id: Uuid,
    pub issue: Uuid,
    pub worktree: String,
    pub body: String,
    pub extra: HashMap<String, toml::Value>,
}

// Internal front matter structs for serde
#[derive(Debug, Serialize, Deserialize)]
struct IssueFrontMatter {
    id: String,
    title: String,
    status: String,
    #[serde(flatten)]
    extra: HashMap<String, toml::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LinkFrontMatter {
    id: String,
    issue: String,
    worktree: String,
    #[serde(flatten)]
    extra: HashMap<String, toml::Value>,
}

fn split_front_matter(content: &str) -> Result<(&str, &str), StoreError> {
    // Expect content starts with "+++\n" or "+++\r\n"
    let after_open = if let Some(rest) = content.strip_prefix("+++\r\n") {
        rest
    } else if let Some(rest) = content.strip_prefix("+++\n") {
        rest
    } else {
        return Err(StoreError::MissingFrontMatter);
    };

    // Find closing delimiter: "\n+++\n", "\n+++\r\n", or "\n+++" at EOF
    if let Some(idx) = after_open.find("\n+++\n") {
        let front = &after_open[..idx];
        let body = &after_open[idx + 5..];
        Ok((front, body))
    } else if let Some(idx) = after_open.find("\n+++\r\n") {
        let front = &after_open[..idx];
        let body = &after_open[idx + 6..];
        Ok((front, body))
    } else if let Some(idx) = after_open.find("\n+++") {
        // Check if it's at the end (no trailing newline) or followed by nothing
        let front = &after_open[..idx];
        let rest = &after_open[idx + 4..];
        // rest is either "" or starts with \r or \n
        let body = if let Some(stripped) = rest.strip_prefix("\r\n") {
            stripped
        } else if let Some(stripped) = rest.strip_prefix('\n') {
            stripped
        } else if rest.is_empty() {
            ""
        } else {
            // Not a delimiter line, treat as missing
            return Err(StoreError::MissingFrontMatter);
        };
        Ok((front, body))
    } else {
        Err(StoreError::MissingFrontMatter)
    }
}

fn parse_status(s: &str) -> Result<Status, StoreError> {
    match s {
        "open" => Ok(Status::Open),
        "closed" => Ok(Status::Closed),
        "archived" => Ok(Status::Archived),
        other => Err(StoreError::InvalidStatus(other.to_string())),
    }
}

// Public parsing from string, used by tests and file loading
pub fn parse_issue_str(content: &str) -> Result<Issue, StoreError> {
    let (front_str, body) = split_front_matter(content)?;
    let fm: IssueFrontMatter = toml::from_str(front_str).map_err(StoreError::InvalidToml)?;
    let id = Uuid::parse_str(&fm.id).map_err(|e| StoreError::InvalidUuid {
        field: "id",
        source: e,
    })?;
    let status = parse_status(&fm.status)?;
    Ok(Issue {
        id,
        title: fm.title,
        status,
        body: body.to_string(),
        extra: fm.extra,
    })
}

pub fn parse_link_str(content: &str) -> Result<Link, StoreError> {
    let (front_str, body) = split_front_matter(content)?;
    let fm: LinkFrontMatter = toml::from_str(front_str).map_err(StoreError::InvalidToml)?;
    let id = Uuid::parse_str(&fm.id).map_err(|e| StoreError::InvalidUuid {
        field: "id",
        source: e,
    })?;
    let issue = Uuid::parse_str(&fm.issue).map_err(|e| StoreError::InvalidUuid {
        field: "issue",
        source: e,
    })?;
    Ok(Link {
        id,
        issue,
        worktree: fm.worktree,
        body: body.to_string(),
        extra: fm.extra,
    })
}

pub fn format_issue(issue: &Issue) -> Result<String, StoreError> {
    let fm = IssueFrontMatter {
        id: issue.id.to_string(),
        title: issue.title.clone(),
        status: issue.status.to_string(),
        extra: issue.extra.clone(),
    };
    let toml_str = toml::to_string(&fm)?;
    let mut out = String::new();
    out.push_str("+++\n");
    out.push_str(&toml_str);
    out.push_str("+++\n");
    out.push_str(&issue.body);
    if !issue.body.is_empty() && !issue.body.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

pub fn format_link(link: &Link) -> Result<String, StoreError> {
    let fm = LinkFrontMatter {
        id: link.id.to_string(),
        issue: link.issue.to_string(),
        worktree: link.worktree.clone(),
        extra: link.extra.clone(),
    };
    let toml_str = toml::to_string(&fm)?;
    let mut out = String::new();
    out.push_str("+++\n");
    out.push_str(&toml_str);
    out.push_str("+++\n");
    out.push_str(&link.body);
    if !link.body.is_empty() && !link.body.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

// File I/O with validation

pub fn read_issue_file(path: &Path) -> Result<Issue, StoreError> {
    let content = fs::read_to_string(path)?;
    let issue = parse_issue_str(&content)?;
    // Validate filename matches id if file stem looks like UUID
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        && let Ok(file_uuid) = Uuid::parse_str(stem)
        && file_uuid != issue.id
    {
        return Err(StoreError::IdMismatch {
            file_stem: stem.to_string(),
            id: issue.id.to_string(),
        });
    }
    Ok(issue)
}

pub fn read_link_file(path: &Path) -> Result<Link, StoreError> {
    let content = fs::read_to_string(path)?;
    let link = parse_link_str(&content)?;
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        && let Ok(file_uuid) = Uuid::parse_str(stem)
        && file_uuid != link.id
    {
        return Err(StoreError::IdMismatch {
            file_stem: stem.to_string(),
            id: link.id.to_string(),
        });
    }
    Ok(link)
}

pub fn write_issue_file(issue: &Issue, dir: &Path) -> Result<PathBuf, StoreError> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.md", issue.id));
    // If file exists, check it is not malformed before overwriting.
    // We allow overwrite only if existing file is valid or does not exist.
    // If existing file is malformed, refuse.
    if path.exists() {
        let existing = fs::read_to_string(&path)?;
        if let Err(e) = parse_issue_str(&existing) {
            return Err(StoreError::RefusingOverwrite {
                path: path.display().to_string(),
                reason: e.to_string(),
            });
        }
    }
    let content = format_issue(issue)?;
    // Atomic write: write to temp then rename
    let tmp = dir.join(format!(".{}.tmp", issue.id));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

pub fn write_link_file(link: &Link, dir: &Path) -> Result<PathBuf, StoreError> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.md", link.id));
    if path.exists() {
        let existing = fs::read_to_string(&path)?;
        if let Err(e) = parse_link_str(&existing) {
            return Err(StoreError::RefusingOverwrite {
                path: path.display().to_string(),
                reason: e.to_string(),
            });
        }
    }
    let content = format_link(link)?;
    let tmp = dir.join(format!(".{}.tmp", link.id));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

pub fn delete_issue_file(id: Uuid, dir: &Path) -> Result<(), StoreError> {
    // Deletion is file-only per spec. This helper is for file-only operation or tests.
    // It does not go through the tracker's write path.
    let path = dir.join(format!("{}.md", id));
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn delete_link_file(id: Uuid, dir: &Path) -> Result<(), StoreError> {
    let path = dir.join(format!("{}.md", id));
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn load_issues(dir: &Path) -> Result<Vec<Issue>, StoreError> {
    let mut issues = Vec::new();
    if !dir.exists() {
        return Ok(issues);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let issue = read_issue_file(&path)?;
        issues.push(issue);
    }
    Ok(issues)
}

pub fn load_links(dir: &Path) -> Result<Vec<Link>, StoreError> {
    let mut links = Vec::new();
    if !dir.exists() {
        return Ok(links);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let link = read_link_file(&path)?;
        links.push(link);
    }
    Ok(links)
}

// Validation helpers

pub fn is_valid_issue_str(content: &str) -> bool {
    parse_issue_str(content).is_ok()
}

pub fn is_valid_link_str(content: &str) -> bool {
    parse_link_str(content).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_issue() -> Issue {
        Issue {
            id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
            title: "Improve startup time".to_string(),
            status: Status::Open,
            body: "Investigate why startup takes several seconds.\n".to_string(),
            extra: HashMap::new(),
        }
    }

    fn sample_link() -> Link {
        Link {
            id: Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
            issue: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
            worktree: "/home/zac/dev/projects/copse-worktrees/main".to_string(),
            body: "".to_string(),
            extra: HashMap::new(),
        }
    }

    #[test]
    fn issue_round_trip() {
        let issue = sample_issue();
        let s = format_issue(&issue).unwrap();
        let parsed = parse_issue_str(&s).unwrap();
        assert_eq!(issue, parsed);
    }

    #[test]
    fn issue_statuses() {
        for status in [Status::Open, Status::Closed, Status::Archived] {
            let mut issue = sample_issue();
            issue.status = status;
            let s = format_issue(&issue).unwrap();
            let parsed = parse_issue_str(&s).unwrap();
            assert_eq!(parsed.status, status);
        }
    }

    #[test]
    fn invalid_status_rejected() {
        let content = "+++\n\
id = \"11111111-1111-4111-8111-111111111111\"\n\
title = \"Test\"\n\
status = \"invalid\"\n\
+++\n\
Body\n";
        assert!(parse_issue_str(content).is_err());
    }

    #[test]
    fn preserves_unknown_fields() {
        let content = "+++\n\
id = \"11111111-1111-4111-8111-111111111111\"\n\
title = \"Test\"\n\
status = \"open\"\n\
custom = \"keep me\"\n\
number = 42\n\
+++\n\
Body\n";
        let parsed = parse_issue_str(content).unwrap();
        assert_eq!(
            parsed.extra.get("custom").unwrap().as_str().unwrap(),
            "keep me"
        );
        assert_eq!(
            parsed.extra.get("number").unwrap().as_integer().unwrap(),
            42
        );
        let formatted = format_issue(&parsed).unwrap();
        let reparsed = parse_issue_str(&formatted).unwrap();
        assert_eq!(reparsed.extra, parsed.extra);
    }

    #[test]
    fn malformed_missing_front_matter() {
        let content = "Just a body without front matter";
        assert!(matches!(
            parse_issue_str(content),
            Err(StoreError::MissingFrontMatter)
        ));
    }

    #[test]
    fn malformed_invalid_toml() {
        let content = "+++\n\
id = \"not-a-uuid\"\n\
title = \"Test\"\n\
status = \"open\"\n\
+++\n\
Body\n";
        assert!(parse_issue_str(content).is_err());
    }

    #[test]
    fn markdown_body_preserved() {
        let mut issue = sample_issue();
        issue.body = "# Heading\n\nSome **markdown** with `code`.\n".to_string();
        let s = format_issue(&issue).unwrap();
        let parsed = parse_issue_str(&s).unwrap();
        assert_eq!(parsed.body, issue.body);
    }

    #[test]
    fn link_round_trip() {
        let link = sample_link();
        let s = format_link(&link).unwrap();
        let parsed = parse_link_str(&s).unwrap();
        assert_eq!(link, parsed);
    }

    #[test]
    fn link_preserves_unknown() {
        let content = "+++\n\
id = \"22222222-2222-4222-8222-222222222222\"\n\
issue = \"11111111-1111-4111-8111-111111111111\"\n\
worktree = \"/tmp/worktree\"\n\
extra = \"value\"\n\
+++\n\
";
        let parsed = parse_link_str(content).unwrap();
        assert_eq!(
            parsed.extra.get("extra").unwrap().as_str().unwrap(),
            "value"
        );
        let formatted = format_link(&parsed).unwrap();
        let reparsed = parse_link_str(&formatted).unwrap();
        assert_eq!(reparsed.extra, parsed.extra);
    }

    #[test]
    fn refuses_to_overwrite_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let path = dir.path().join(format!("{}.md", id));
        std::fs::write(&path, "not valid toml").unwrap();
        let issue = Issue {
            id,
            title: "New".to_string(),
            status: Status::Open,
            body: "Body".to_string(),
            extra: HashMap::new(),
        };
        let err = write_issue_file(&issue, dir.path()).unwrap_err();
        assert!(matches!(err, StoreError::RefusingOverwrite { .. }));
        // Original file unchanged
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not valid toml");
    }

    #[test]
    fn id_mismatch_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap();
        let other_id = Uuid::parse_str("44444444-4444-4444-8444-444444444444").unwrap();
        let issue = Issue {
            id: other_id,
            title: "Test".to_string(),
            status: Status::Open,
            body: "".to_string(),
            extra: HashMap::new(),
        };
        let content = format_issue(&issue).unwrap();
        let path = dir.path().join(format!("{}.md", id));
        std::fs::write(&path, content).unwrap();
        let err = read_issue_file(&path).unwrap_err();
        assert!(matches!(err, StoreError::IdMismatch { .. }));
    }

    #[test]
    fn scan_only_md_files() {
        let dir = tempfile::tempdir().unwrap();
        let issue = sample_issue();
        write_issue_file(&issue, dir.path()).unwrap();
        // Add a non-md file that should be ignored
        std::fs::write(dir.path().join("README.txt"), "ignored").unwrap();
        std::fs::write(dir.path().join("other.md.bak"), "ignored").unwrap();
        let issues = load_issues(dir.path()).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, issue.id);
    }

    #[test]
    fn empty_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let issues = load_issues(dir.path()).unwrap();
        assert!(issues.is_empty());
        let links = load_links(dir.path()).unwrap();
        assert!(links.is_empty());
    }

    #[test]
    fn missing_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nonexistent");
        let issues = load_issues(&missing).unwrap();
        assert!(issues.is_empty());
    }
}
