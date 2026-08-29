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
A GitHub Issue that records project work in Copse's issue tracker.
_Avoid_: Ticket, task

**Link**:
An explicit association between a worktree, an issue, and optionally an agent.
_Avoid_: Inferred assignment, convention

**Wayfinder map**:
A GitHub Issue that indexes an effort's decision issues and their blocking relationships. Copse renders it without changing it.
_Avoid_: Roadmap, plan

**Worktree**:
A Git worktree in the board's repository.
_Avoid_: Checkout, branch
