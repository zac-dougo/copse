#![allow(dead_code)]

use std::path::{Component, Path, PathBuf};
use std::process::Command;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("not a git repository: {path}")]
    NotGitRepo { path: String },
    #[error("git command failed: {cmd}: {stderr}")]
    GitFailed { cmd: String, stderr: String },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse worktree list: {0}")]
    ParseError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    pub is_bare: bool,
    pub is_detached: bool,
    pub is_prunable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardRepository {
    /// Primary worktree path. Owns `.copse`.
    pub primary_path: PathBuf,
    /// The worktree that contains the starting directory.
    pub current_worktree_path: PathBuf,
    /// All worktrees in the repo.
    pub worktrees: Vec<Worktree>,
    /// Primary-owned tracker dir (worktree links only; issues live on GitHub).
    pub copse_dir: PathBuf,
    pub links_dir: PathBuf,
    pub is_copse_present: bool,
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, DiscoveryError> {
    let mut cmd = Command::new("git");
    cmd.arg("-C");
    cmd.arg(cwd);
    cmd.args(args);
    let output = cmd.output().map_err(DiscoveryError::Io)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let cmd_str = format!("git -C {} {}", cwd.display(), args.join(" "));
        return Err(DiscoveryError::GitFailed {
            cmd: cmd_str,
            stderr,
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(stdout)
}

fn is_inside_work_tree(cwd: &Path) -> bool {
    // Use git rev-parse --is-inside-work-tree which prints "true" or "false"
    // If git fails, treat as not inside.
    match git_output(cwd, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(out) => out.trim() == "true",
        Err(_) => false,
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut buf = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                buf.pop();
            }
            Component::CurDir => {}
            _ => buf.push(comp.as_os_str()),
        }
    }
    buf
}

fn show_toplevel(cwd: &Path) -> Result<PathBuf, DiscoveryError> {
    let out = git_output(cwd, &["rev-parse", "--show-toplevel"])?;
    let path = out.trim();
    if path.is_empty() {
        return Err(DiscoveryError::ParseError("empty toplevel".to_string()));
    }
    let p = PathBuf::from(path);
    Ok(p.canonicalize().unwrap_or_else(|_| normalize(&p)))
}

fn git_common_dir(cwd: &Path) -> Result<PathBuf, DiscoveryError> {
    let out = git_output(cwd, &["rev-parse", "--git-common-dir"])?;
    let raw = out.trim();
    if raw.is_empty() {
        return Err(DiscoveryError::ParseError(
            "empty git-common-dir".to_string(),
        ));
    }
    let p = PathBuf::from(raw);
    let abs = if p.is_absolute() { p } else { cwd.join(p) };
    let cleaned = normalize(&abs);
    Ok(cleaned.canonicalize().unwrap_or(cleaned))
}

fn worktree_list(cwd: &Path) -> Result<Vec<Worktree>, DiscoveryError> {
    let out = git_output(cwd, &["worktree", "list", "--porcelain"])?;
    parse_worktree_porcelain(&out)
}

fn parse_worktree_porcelain(s: &str) -> Result<Vec<Worktree>, DiscoveryError> {
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;

    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() {
            if let Some(wt) = current.take() {
                worktrees.push(wt);
            }
            continue;
        }
        if let Some(path_str) = line.strip_prefix("worktree ") {
            if let Some(prev) = current.take() {
                worktrees.push(prev);
            }
            current = Some(Worktree {
                path: PathBuf::from(path_str),
                head: String::new(),
                branch: None,
                is_bare: false,
                is_detached: false,
                is_prunable: false,
            });
        } else if let Some(head) = line.strip_prefix("HEAD ") {
            if let Some(ref mut wt) = current {
                wt.head = head.to_string();
            }
        } else if let Some(branch) = line.strip_prefix("branch ") {
            if let Some(ref mut wt) = current {
                wt.branch = Some(branch.to_string());
            }
        } else if line == "bare" {
            if let Some(ref mut wt) = current {
                wt.is_bare = true;
            }
        } else if line == "detached" {
            if let Some(ref mut wt) = current {
                wt.is_detached = true;
            }
        } else if line == "prunable" || line.starts_with("prunable ") {
            if let Some(ref mut wt) = current {
                wt.is_prunable = true;
            }
        } else if line.starts_with("locked") {
            // Ignore locked reason.
        } else {
            // Unknown line, ignore for forward compat (e.g., "is_bare").
            // Some porcelains emit "is_bare" differently.
            if line == "is_bare"
                && let Some(ref mut wt) = current
            {
                wt.is_bare = true;
            }
        }
    }
    if let Some(wt) = current {
        worktrees.push(wt);
    }

    if worktrees.is_empty() && !s.trim().is_empty() {
        return Err(DiscoveryError::ParseError(format!(
            "no worktrees parsed from: {s}"
        )));
    }

    Ok(worktrees)
}

pub fn discover(cwd: &Path) -> Result<BoardRepository, DiscoveryError> {
    if !cwd.exists() {
        return Err(DiscoveryError::NotGitRepo {
            path: cwd.display().to_string(),
        });
    }

    if !is_inside_work_tree(cwd) {
        return Err(DiscoveryError::NotGitRepo {
            path: cwd.display().to_string(),
        });
    }

    let current_worktree_path = show_toplevel(cwd)?;
    let common_dir = git_common_dir(cwd)?;
    // common_dir is .../.git . Primary is parent of common_dir.
    let primary_path = common_dir
        .parent()
        .map(PathBuf::from)
        .unwrap_or(common_dir.clone());
    let primary_path = primary_path
        .canonicalize()
        .unwrap_or_else(|_| normalize(&primary_path));

    // Canonicalize primary and current for comparison, but keep as returned from git.
    // Worktree list is authoritative for all worktrees.
    let worktrees = worktree_list(cwd)?;

    // Find primary worktree entry that matches primary_path, fallback to first non-bare.
    let _primary_in_list = worktrees.iter().find(|wt| wt.path == primary_path);

    let copse_dir = primary_path.join(".copse");
    let links_dir = copse_dir.join("links");
    let is_copse_present = copse_dir.exists();

    Ok(BoardRepository {
        primary_path,
        current_worktree_path,
        worktrees,
        copse_dir,
        links_dir,
        is_copse_present,
    })
}

// Helpers for tests and callers that need branch short name.
pub fn branch_short_name(full: &str) -> Option<String> {
    full.strip_prefix("refs/heads/").map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_repo(path: &Path) {
        run_git(path, &["init"]);
        run_git(path, &["config", "user.email", "test@test.com"]);
        run_git(path, &["config", "user.name", "Test"]);
        fs::write(path.join("README.md"), "# test\n").unwrap();
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", "initial"]);
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .expect("git command failed");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn create_worktree(repo: &Path, name: &str, branch: &str) -> PathBuf {
        let wt_path = repo.parent().unwrap().join(format!(
            "{}-{}",
            repo.file_name().unwrap().to_str().unwrap(),
            name
        ));
        run_git(
            repo,
            &["worktree", "add", wt_path.to_str().unwrap(), "-b", branch],
        );
        wt_path
    }

    #[test]
    fn nested_directory_resolves_to_containing_worktree() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        init_repo(&repo);
        fs::create_dir_all(repo.join("docs/nested")).unwrap();
        let nested = repo.join("docs/nested");
        let board = discover(&nested).unwrap();
        assert_eq!(board.current_worktree_path, repo);
        assert_eq!(board.primary_path, repo);
        assert_eq!(board.worktrees.len(), 1);
    }

    #[test]
    fn linked_worktree_opens_same_board() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        init_repo(&repo);
        let wt_path = create_worktree(&repo, "second", "feature/second");

        // Discover from primary
        let from_primary = discover(&repo).unwrap();
        // Discover from linked
        let from_linked = discover(&wt_path).unwrap();

        assert_eq!(from_primary.primary_path, from_linked.primary_path);
        assert_eq!(from_primary.primary_path, repo);
        assert_eq!(from_primary.worktrees.len(), 2);
        assert_eq!(from_linked.worktrees.len(), 2);
        // Current worktree differs
        assert_eq!(from_primary.current_worktree_path, repo);
        assert_eq!(from_linked.current_worktree_path, wt_path);
        // Both see same copse dir (primary-owned)
        assert_eq!(from_primary.copse_dir, from_linked.copse_dir);
        assert_eq!(from_primary.copse_dir, repo.join(".copse"));
    }

    #[test]
    fn copse_presence_detected() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        init_repo(&repo);
        let board = discover(&repo).unwrap();
        assert!(!board.is_copse_present);
        fs::create_dir_all(repo.join(".copse/links")).unwrap();
        let board2 = discover(&repo).unwrap();
        assert!(board2.is_copse_present);
    }

    #[test]
    fn primary_owns_copse() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        init_repo(&repo);
        let wt_path = create_worktree(&repo, "second", "feature/second");
        // Create .copse only in primary
        fs::create_dir_all(repo.join(".copse")).unwrap();
        let board = discover(&wt_path).unwrap();
        assert_eq!(board.copse_dir, repo.join(".copse"));
        // No .copse in linked worktree
        assert!(!wt_path.join(".copse").exists());
    }

    #[test]
    fn outside_git_fails() {
        let tmp = TempDir::new().unwrap();
        let outside = tmp.path().join("not-a-repo");
        fs::create_dir(&outside).unwrap();
        let err = discover(&outside).unwrap_err();
        assert!(matches!(err, DiscoveryError::NotGitRepo { .. }));
    }

    #[test]
    fn nonexistent_path_fails() {
        let err = discover(Path::new("/tmp/does-not-exist-12345")).unwrap_err();
        assert!(matches!(err, DiscoveryError::NotGitRepo { .. }));
    }

    #[test]
    fn worktree_list_parsing() {
        let sample = "worktree /tmp/repo\nHEAD abc123\nbranch refs/heads/main\n\nworktree /tmp/repo-second\nHEAD def456\nbranch refs/heads/feature\n\n";
        let list = parse_worktree_porcelain(sample).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].path, PathBuf::from("/tmp/repo"));
        assert_eq!(list[0].branch, Some("refs/heads/main".to_string()));
        assert_eq!(list[1].branch, Some("refs/heads/feature".to_string()));
    }

    #[test]
    fn handles_prunable_and_detached() {
        let sample = "worktree /tmp/repo\nHEAD abc123\nbranch refs/heads/main\n\nworktree /tmp/old\nHEAD abc123\ndetached\nprunable\n\n";
        let list = parse_worktree_porcelain(sample).unwrap();
        assert_eq!(list.len(), 2);
        assert!(!list[0].is_detached);
        assert!(list[1].is_detached);
        assert!(list[1].is_prunable);
    }

    #[test]
    fn branch_short_name_works() {
        assert_eq!(
            branch_short_name("refs/heads/main"),
            Some("main".to_string())
        );
        assert_eq!(
            branch_short_name("refs/heads/feature/second"),
            Some("feature/second".to_string())
        );
        assert_eq!(branch_short_name("HEAD"), None);
    }
}
