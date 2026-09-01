# Copse

Copse is a terminal application for tracking work in one Git repository while its work happens across multiple worktrees.

## Language

**Board**:
The read-only Copse view of one repository's worktrees, issues, and Wayfinder maps.
_Avoid_: Dashboard, workspace

**Agent**:
A Herdr-recognized coding-agent process in the active local Herdr session. Copse reads its state but does not control it.
_Avoid_: Worker, session

**Issue**:
A local record of work in `.copse/issues/*.md` (and, for GitHub-backed repos, a GitHub Issue). Every Issue has an `id`, `title`, `status` (`open`/`closed`/`archived`), and optional triage labels.
_Avoid_: Ticket, task

**Frontier**:
An open, unblocked, unassigned Issue. It is ready to be claimed.
_Avoid_: Ready, todo

**Blocked**:
An open Issue that has at least one open blocker referenced in its `Blocked by` line.
_Avoid_: Waiting

**Assigned**:
An open Issue that is linked to a worktree via `.copse/links/` (or, on GitHub, assigned to a user) and has no open blockers. An Agent may be working on it.
_Avoid_: Working, claimed, in-progress

**Done**:
A closed or archived Issue.
_Avoid_: Completed, resolved

**Link**:
An explicit association between a worktree, an issue, and optionally an agent.
_Avoid_: Inferred assignment, convention

**Wayfinder map**:
A GitHub Issue that indexes an effort's decision issues and their blocking relationships. Copse renders it without changing it.
_Avoid_: Roadmap, plan

**Worktree**:
A Git worktree in the board's repository.
_Avoid_: Checkout, branch
