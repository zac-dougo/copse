#![allow(dead_code)]

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Idle => write!(f, "idle"),
            AgentStatus::Working => write!(f, "working"),
            AgentStatus::Blocked => write!(f, "blocked"),
            AgentStatus::Done => write!(f, "done"),
            AgentStatus::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub agent: String,
    pub agent_status: AgentStatus,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default)]
    pub focused: Option<bool>,
    // Extra fields ignored via serde default.
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: Option<AgentStatus>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    pub workspace_id: String,
    pub tab_id: String,
    #[serde(default)]
    pub focused: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    pub workspace_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub agent_status: Option<AgentStatus>,
    #[serde(default)]
    pub focused: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub agents: Vec<Agent>,
    #[serde(default)]
    pub panes: Vec<Pane>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub focused_workspace_id: Option<String>,
    #[serde(default)]
    pub focused_tab_id: Option<String>,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub protocol: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct SnapshotResult {
    snapshot: Snapshot,
}

#[derive(Debug, Clone, Deserialize)]
struct SnapshotOuter {
    result: SnapshotResult,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    outer_type: Option<String>,
}

#[derive(Debug, Error)]
pub enum HerdrError {
    #[error("herdr not found: {0}")]
    NotFound(String),
    #[error("herdr command failed: {0}")]
    CommandFailed(String),
    #[error("failed to parse snapshot: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl Snapshot {
    pub fn from_json(s: &str) -> Result<Self, HerdrError> {
        // Try outer wrapper first, then direct.
        if let Ok(outer) = serde_json::from_str::<SnapshotOuter>(s) {
            return Ok(outer.result.snapshot);
        }
        // Fallback: try direct Snapshot, or {result: {snapshot}} without outer id/type
        if let Ok(wrapped) = serde_json::from_str::<SnapshotResult>(s) {
            return Ok(wrapped.snapshot);
        }
        let snap = serde_json::from_str::<Snapshot>(s)?;
        Ok(snap)
    }
}

pub async fn fetch_snapshot() -> Result<Snapshot, HerdrError> {
    use tokio::process::Command;
    let output = Command::new("herdr")
        .args(["api", "snapshot"])
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                HerdrError::NotFound(e.to_string())
            } else {
                HerdrError::Io(e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(HerdrError::CommandFailed(stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Snapshot::from_json(&stdout)
}

pub fn fetch_snapshot_blocking() -> Result<Snapshot, HerdrError> {
    use std::process::Command;
    let output = Command::new("herdr")
        .args(["api", "snapshot"])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                HerdrError::NotFound(e.to_string())
            } else {
                HerdrError::Io(e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(HerdrError::CommandFailed(stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Snapshot::from_json(&stdout)
}

// Map an agent's foreground_cwd to a worktree path by prefix matching.
// Returns the longest matching worktree path, or None.
pub fn map_agent_to_worktree(
    agent: &Agent,
    worktree_paths: &[std::path::PathBuf],
) -> Option<std::path::PathBuf> {
    let cwd_str = agent.foreground_cwd.as_ref().or(agent.cwd.as_ref())?;
    let cwd = std::path::Path::new(cwd_str);
    let mut best: Option<&std::path::PathBuf> = None;
    let mut best_len = 0usize;
    for wt in worktree_paths {
        if cwd.starts_with(wt) {
            let len = wt.as_os_str().len();
            if len > best_len {
                best = Some(wt);
                best_len = len;
            }
        }
    }
    best.cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SAMPLE: &str = r#"{"id":"cli:api:snapshot","result":{"snapshot":{"agents":[{"agent":"pi","agent_session":{"agent":"pi","kind":"path","source":"herdr:pi","value":"/home/zac/.pi/agent/sessions/test.jsonl"},"agent_status":"working","cwd":"/home/zac/dev/projects/copse-worktrees/main","focused":true,"foreground_cwd":"/home/zac/dev/projects/copse-worktrees/main","pane_id":"w5:p1","revision":14,"screen_detection_skipped":true,"state_change_seq":202,"tab_id":"w5:t1","terminal_id":"term_123","terminal_title":"π - copse","terminal_title_stripped":"π - copse","workspace_id":"w5"}],"focused_pane_id":"w5:p1","focused_tab_id":"w5:t1","focused_workspace_id":"w5","layouts":[],"panes":[{"agent":"pi","agent_status":"working","cwd":"/home/zac/dev/projects/copse-worktrees/main","focused":true,"foreground_cwd":"/home/zac/dev/projects/copse-worktrees/main","pane_id":"w5:p1","tab_id":"w5:t1","terminal_id":"term_123","workspace_id":"w5"}],"protocol":20,"tabs":[],"version":"0.8.2","workspaces":[{"active_tab_id":"w5:t1","agent_status":"working","focused":true,"label":"main","number":1,"pane_count":1,"tab_count":1,"workspace_id":"w5"}]},"type":"session_snapshot"}}"#;

    #[test]
    fn parses_snapshot_outer() {
        let snap = Snapshot::from_json(SAMPLE).unwrap();
        assert_eq!(snap.agents.len(), 1);
        assert_eq!(snap.agents[0].agent, "pi");
        assert_eq!(snap.agents[0].agent_status, AgentStatus::Working);
        assert_eq!(snap.agents[0].pane_id, "w5:p1");
        assert_eq!(
            snap.agents[0].foreground_cwd.as_deref(),
            Some("/home/zac/dev/projects/copse-worktrees/main")
        );
        assert_eq!(snap.focused_pane_id.as_deref(), Some("w5:p1"));
        assert_eq!(snap.version.as_deref(), Some("0.8.2"));
    }

    #[test]
    fn parses_all_statuses() {
        for (s, expected) in [
            ("\"idle\"", AgentStatus::Idle),
            ("\"working\"", AgentStatus::Working),
            ("\"blocked\"", AgentStatus::Blocked),
            ("\"done\"", AgentStatus::Done),
            ("\"unknown\"", AgentStatus::Unknown),
        ] {
            let status: AgentStatus = serde_json::from_str(s).unwrap();
            assert_eq!(status, expected);
        }
    }

    #[test]
    fn parses_minimal_snapshot() {
        let json = r#"{"agents":[],"panes":[],"workspaces":[]}"#;
        let snap = Snapshot::from_json(json).unwrap();
        assert!(snap.agents.is_empty());
    }

    #[test]
    fn parses_result_wrapper() {
        let json = r#"{"result":{"snapshot":{"agents":[{"agent":"pi","agent_status":"idle","pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1"}],"panes":[],"workspaces":[]}}}"#;
        let snap = Snapshot::from_json(json).unwrap();
        assert_eq!(snap.agents.len(), 1);
        assert_eq!(snap.agents[0].agent_status, AgentStatus::Idle);
    }

    #[test]
    fn maps_agent_to_worktree() {
        let agent = Agent {
            agent: "pi".to_string(),
            agent_status: AgentStatus::Working,
            cwd: Some("/home/zac".to_string()),
            foreground_cwd: Some("/home/zac/dev/projects/copse-worktrees/main/src".to_string()),
            pane_id: "w5:p1".to_string(),
            workspace_id: "w5".to_string(),
            tab_id: "w5:t1".to_string(),
            terminal_id: None,
            focused: None,
        };
        let wts = vec![
            PathBuf::from("/home/zac/dev/projects/copse-worktrees/main"),
            PathBuf::from("/home/zac/dev/projects/copse-worktrees/second"),
        ];
        let mapped = map_agent_to_worktree(&agent, &wts).unwrap();
        assert_eq!(
            mapped,
            PathBuf::from("/home/zac/dev/projects/copse-worktrees/main")
        );
    }

    #[test]
    fn maps_agent_no_match() {
        let agent = Agent {
            agent: "pi".to_string(),
            agent_status: AgentStatus::Working,
            cwd: Some("/tmp/other".to_string()),
            foreground_cwd: Some("/tmp/other".to_string()),
            pane_id: "w5:p1".to_string(),
            workspace_id: "w5".to_string(),
            tab_id: "w5:t1".to_string(),
            terminal_id: None,
            focused: None,
        };
        let wts = vec![PathBuf::from("/home/zac/dev/projects/copse")];
        assert!(map_agent_to_worktree(&agent, &wts).is_none());
    }

    #[test]
    fn live_snapshot_parses() {
        // This test runs only if herdr is available. It should not fail CI if herdr is missing.
        // We attempt a blocking fetch and just check it doesn't panic.
        if std::process::Command::new("herdr")
            .args(["api", "snapshot"])
            .output()
            .is_ok()
        {
            if let Ok(snap) = fetch_snapshot_blocking() {
                // Snapshot should have at least the version field
                assert!(snap.version.is_some() || snap.protocol.is_some());
            }
        }
    }
}
