#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

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
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    #[error("invalid issue number for {field}: {value}")]
    InvalidIssueNumber { field: &'static str, value: String },
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

/// An explicit association between a worktree and a GitHub issue.
///
/// Issues themselves live on GitHub. The link only records which issue number
/// a worktree is working on. `id` is a local UUID that names the file;
/// `issue` is the GitHub issue number.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub id: Uuid,
    pub issue: u64,
    pub worktree: String,
    pub body: String,
    pub extra: HashMap<String, toml::Value>,
}

// Internal front matter struct for serde
#[derive(Debug, Serialize, Deserialize)]
struct LinkFrontMatter {
    id: String,
    issue: u64,
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

fn parse_issue_number(value: &toml::Value) -> Result<u64, StoreError> {
    match value {
        toml::Value::Integer(n) if *n >= 0 => Ok(*n as u64),
        // A quoted number is still a number.
        toml::Value::String(s) => s
            .parse::<u64>()
            .map_err(|_| StoreError::InvalidIssueNumber {
                field: "issue",
                value: s.clone(),
            }),
        other => Err(StoreError::InvalidIssueNumber {
            field: "issue",
            value: other.to_string(),
        }),
    }
}

// Public parsing from string, used by tests and file loading.
// Accepts both `issue = 123` and `issue = "123"`; rejects UUID-era links
// (`issue = "<uuid>"`) so stale local-tracker links fail loudly instead of
// silently pointing nowhere.
pub fn parse_link_str(content: &str) -> Result<Link, StoreError> {
    let (front_str, body) = split_front_matter(content)?;
    let table: toml::Table = toml::from_str(front_str).map_err(StoreError::InvalidToml)?;
    let mut table = table;
    let id_str = table
        .remove("id")
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or(StoreError::MissingField("id"))?;
    let id = Uuid::parse_str(&id_str).map_err(|e| StoreError::InvalidUuid {
        field: "id",
        source: e,
    })?;
    let issue_value = table
        .remove("issue")
        .ok_or_else(|| StoreError::InvalidIssueNumber {
            field: "issue",
            value: String::from("(missing)"),
        })?;
    let issue = parse_issue_number(&issue_value)?;
    let worktree = table
        .remove("worktree")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let extra: HashMap<String, toml::Value> = table.into_iter().collect();
    Ok(Link {
        id,
        issue,
        worktree,
        body: body.to_string(),
        extra,
    })
}

pub fn format_link(link: &Link) -> Result<String, StoreError> {
    let fm = LinkFrontMatter {
        id: link.id.to_string(),
        issue: link.issue,
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

pub fn delete_link_file(id: Uuid, dir: &Path) -> Result<(), StoreError> {
    let path = dir.join(format!("{}.md", id));
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
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

// Validation helper

pub fn is_valid_link_str(content: &str) -> bool {
    parse_link_str(content).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_link() -> Link {
        Link {
            id: Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
            issue: 42,
            worktree: "/home/zac/dev/projects/copse-worktrees/main".to_string(),
            body: "".to_string(),
            extra: HashMap::new(),
        }
    }

    #[test]
    fn link_round_trip() {
        let link = sample_link();
        let s = format_link(&link).unwrap();
        let parsed = parse_link_str(&s).unwrap();
        assert_eq!(link, parsed);
    }

    #[test]
    fn link_accepts_quoted_number() {
        let content = "+++\n\
id = \"22222222-2222-4222-8222-222222222222\"\n\
issue = \"42\"\n\
worktree = \"/tmp/worktree\"\n\
+++\n\
";
        let parsed = parse_link_str(content).unwrap();
        assert_eq!(parsed.issue, 42);
    }

    #[test]
    fn link_rejects_uuid_era_issue() {
        // Links from the local-tracker era pointed at UUIDs. They must fail
        // loudly so nobody silently works against a dead link.
        let content = "+++\n\
id = \"22222222-2222-4222-8222-222222222222\"\n\
issue = \"11111111-1111-4111-8111-111111111111\"\n\
worktree = \"/tmp/worktree\"\n\
+++\n\
";
        assert!(matches!(
            parse_link_str(content),
            Err(StoreError::InvalidIssueNumber { .. })
        ));
    }

    #[test]
    fn link_rejects_negative_issue() {
        let content = "+++\n\
id = \"22222222-2222-4222-8222-222222222222\"\n\
issue = -1\n\
worktree = \"/tmp/worktree\"\n\
+++\n\
";
        assert!(parse_link_str(content).is_err());
    }

    #[test]
    fn link_preserves_unknown() {
        let content = "+++\n\
id = \"22222222-2222-4222-8222-222222222222\"\n\
issue = 42\n\
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
        let link = Link {
            id,
            issue: 7,
            worktree: "/tmp/wt".to_string(),
            body: "".to_string(),
            extra: HashMap::new(),
        };
        let err = write_link_file(&link, dir.path()).unwrap_err();
        assert!(matches!(err, StoreError::RefusingOverwrite { .. }));
        // Original file unchanged
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not valid toml");
    }

    #[test]
    fn id_mismatch_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap();
        let other_id = Uuid::parse_str("44444444-4444-4444-8444-444444444444").unwrap();
        let link = Link {
            id: other_id,
            issue: 7,
            worktree: "/tmp/wt".to_string(),
            body: "".to_string(),
            extra: HashMap::new(),
        };
        let content = format_link(&link).unwrap();
        let path = dir.path().join(format!("{}.md", id));
        std::fs::write(&path, content).unwrap();
        let err = read_link_file(&path).unwrap_err();
        assert!(matches!(err, StoreError::IdMismatch { .. }));
    }

    #[test]
    fn scan_only_md_files() {
        let dir = tempfile::tempdir().unwrap();
        let link = sample_link();
        write_link_file(&link, dir.path()).unwrap();
        // Add non-md files that should be ignored
        std::fs::write(dir.path().join("README.txt"), "ignored").unwrap();
        std::fs::write(dir.path().join("other.md.bak"), "ignored").unwrap();
        let links = load_links(dir.path()).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].id, link.id);
    }

    #[test]
    fn empty_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let links = load_links(dir.path()).unwrap();
        assert!(links.is_empty());
    }

    #[test]
    fn missing_dir_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nonexistent");
        let links = load_links(&missing).unwrap();
        assert!(links.is_empty());
    }

    #[test]
    fn malformed_missing_front_matter() {
        let content = "Just a body without front matter";
        assert!(matches!(
            parse_link_str(content),
            Err(StoreError::MissingFrontMatter)
        ));
    }
}
