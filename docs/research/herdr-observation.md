# Herdr observation for Copse

Research ticket: [.scratch/copse-v1/issues/03-research-herdr-observation.md](../../.scratch/copse-v1/issues/03-research-herdr-observation.md)
Herdr version probed: 0.8.2 (stable channel, protocol 20), default local session

Sources (first-party only):

- `herdr --skill`: the installed agent skill file (195 lines)
- `herdr --help` and the command-group helps: `herdr agent`, `herdr api`, `herdr pane`, `herdr tab`, `herdr workspace`, `herdr worktree`, `herdr session`
- `herdr api schema --json`: JSON Schema for the socket protocol
- Live probes against the running default session

## Question

How Copse can identify the active local session's recognized agents, their states, and their working directories or worktrees, using only first-party Herdr sources, and what of that behaviour is available outside a Herdr-managed pane.

## Findings

### The read-only surface works outside a Herdr-managed pane

The `herdr` binary is a client of a local server. `herdr status` reports the server state and its socket path (`/home/zac/.config/herdr/herdr.sock` in this install):

```
server:
  status: running
  version: 0.8.2
  socket: /home/zac/.config/herdr/herdr.sock
```

`HERDR_ENV=1` marks a process inside a managed pane, alongside `HERDR_WORKSPACE_ID`, `HERDR_TAB_ID`, and `HERDR_PANE_ID`. The skill says: "Herdr injects the caller's context into each managed pane." Outside a pane these variables are absent.

The skill's guardrail, "Do not inspect or control the focused Herdr session from outside Herdr", is addressed to agents running the skill. It does not gate the binary. With `HERDR_ENV` unset, the following all returned data: `herdr agent list`, `herdr api snapshot`, `herdr status`, `herdr worktree list --cwd <path>`. So the observation surface works from a plain terminal. Copse is not an agent and runs only the read-only commands below; it should never run control commands (start, prompt, focus, send-keys, workspace create/close, worktree create/remove, server stop).

### Agents and their states: `herdr agent list`

Output is JSON. Each entry in `result.agents[]` (AgentInfo shape from the schema) looks like:

```json
{
  "agent": "pi",
  "agent_status": "working",
  "cwd": "/home/zac",
  "foreground_cwd": "/home/zac/dev/projects/copse",
  "pane_id": "w5:p1",
  "workspace_id": "w5",
  "tab_id": "w5:t1",
  "terminal_id": "term_65a2c3c1413485",
  "focused": true,
  "revision": 9,
  "state_change_seq": 124,
  "screen_detection_skipped": true,
  "agent_session": {
    "agent": "pi",
    "kind": "path",
    "source": "herdr:pi",
    "value": "/home/zac/.pi/agent/sessions/--home-zac--/2026-08-29T10-49-31-986Z_01a04d23-9251-7836-89eb-7ae2866d7101.jsonl"
  },
  "terminal_title": "\u03c0 - zac",
  "terminal_title_stripped": "\u03c0 - zac"
}
```

The `agent_status` enum comes from `herdr api schema --json` (AgentStatus): `idle`, `working`, `blocked`, `done`, `unknown`. The skill defines the semantics:

- `idle`: ready for input and its tab has been seen in the focused Herdr UI
- `done`: the same underlying idle state, after unseen background work finishes
- `blocked`: Herdr recognized an approval or question UI
- `unknown`: an agent is present but Herdr cannot classify it confidently; it does not prove completion

The skill also notes "CLI reads do not mark it seen", so polling does not disturb the idle/done distinction.

Targeting: the skill says agent commands accept "a unique live agent name or the pane ID currently hosting that agent". In probes on 0.8.2 this held only for pane IDs. `herdr agent get pi` (and `read`, `explain` by name) returned `agent_not_found` with exit 1, even though `agent list` reported the live name as `pi`. `herdr agent get w5:p1` and `herdr agent read w5:p1` worked. Names follow the current pane occupant and can be cleared or replaced; pane IDs are stable within a workspace ("Closed tab and pane IDs are not reused", and a pane moved between workspaces gets a new workspace-qualified ID). Copse should read `agent list` as the source of truth and use pane IDs as handles.

### Working directories

Agents carry two paths: `cwd` (the shell's working directory) and `foreground_cwd` (the working directory of the foreground process; for an agent occupying the pane, that is the agent's directory). The schema marks both nullable strings. In the probe, `cwd` was `/home/zac` and `foreground_cwd` was `/home/zac/dev/projects/copse`, the repo.

`herdr pane list --workspace <ID>` returns the same agent fields per pane plus `scroll` state. Panes without agents exist: the PaneInfo schema makes `agent`, `agent_session`, `cwd`, `foreground_cwd`, `display_agent`, `title`, and the titles nullable. A pane is a terminal location whether or not an agent occupies it.

Workspaces and tabs carry no path. `herdr workspace list` / `herdr workspace get <ID>` return label, number, pane_count, tab_count, agent_status, focused, and active_tab_id only. The path lives on the pane, so workspace-to-repo mapping goes pane cwd to git repo, not the other way.

### Worktrees: `herdr worktree list`

`herdr worktree list --cwd <path>` returns repo identity plus its worktrees:

```json
{
  "result": {
    "source": {
      "repo_key": "/home/zac/dev/projects/copse/.git",
      "repo_name": "copse",
      "repo_root": "/home/zac/dev/projects/copse",
      "source_checkout_path": "/home/zac/dev/projects/copse"
    },
    "worktrees": [
      {
        "branch": "master",
        "is_bare": false,
        "is_detached": false,
        "is_linked_worktree": false,
        "is_prunable": false,
        "label": "copse",
        "path": "/home/zac/dev/projects/copse"
      }
    ]
  }
}
```

`herdr worktree list --workspace <ID>` requires the workspace to sit inside a git work tree; otherwise it errors with `not_git_worktree` (observed on a workspace whose cwd was `/home/zac`). The schema's event list includes `worktree_created`, `worktree_opened`, and `worktree_removed`, so the server tracks worktree changes as events.

To map an agent to a repo: take the agent's `foreground_cwd` (or `cwd`), run `herdr worktree list --cwd <that path>`, and read the `worktrees` entries for branch and path plus the `source` block for the repo root and its checkout.

### Whole-session snapshot: `herdr api snapshot`

One JSON document covers the session: `agents[]`, `panes[]`, `workspaces[]`, `tabs[]`, `layouts[]`, `focused_workspace_id`, `focused_tab_id`, `focused_pane_id`, plus `version` and `protocol`. This is the single read Copse needs to render a board: agents with states and paths, the pane topology, and the focused IDs.

`herdr api schema [--json | --output PATH]` prints the socket protocol schema (schemas: error_response, event, request, subscription_event, success_response). It documents every field above and the server events (`pane_agent_detected`, `pane_agent_status_changed`, `pane_updated`, `pane_output_changed`, plus workspace, tab, layout, and worktree events) and the subscription events (`pane.agent_status_changed`, `pane.output_matched`, `pane.scroll_changed`). The CLI exposes no subscribe command, so live updates from the CLI mean polling. A push channel would mean speaking the socket protocol directly against the schema; that is beyond the CLI and not needed for a v1 read-only board.

### Sessions and errors

`herdr session list [--json]` enumerates running sessions with directory and socket. The CLI talks to the default session's socket. Named sessions and remote sessions are out of scope for Copse per the effort map.

Error shape, from the skill: "CLI server errors are JSON on stderr with exit status 1. CLI syntax errors exit with status 2." Server-side errors observed in probes look like `{"error":{"code":"agent_not_found","message":"agent target pi not found"},"id":"cli:agent:get"}` with exit 1, and `not_git_worktree` for worktree commands outside a git work tree. Copse should parse JSON errors and check exit codes.

Not tested: behaviour when the server is stopped. The skill forbids stopping an active session's server, so the failure mode was not probed. `herdr status` reports server state; read commands against a stopped server would fail to connect.

## Implications for Copse

- Observation primitive: run `herdr api snapshot` with the `herdr` binary from PATH. It needs the local default-session server running. Everything else is derived from that document.
- Agent identity, state, and paths come from the snapshot's `agents[]`. Use pane IDs as handles; do not target agents by name.
- Worktree mapping: for each agent, `herdr worktree list --cwd <foreground_cwd>` gives the repo root, branch, and worktree path.
- Read-only only. The snapshot command and `worktree list` are non-mutating. None of the control commands (start, prompt, send-keys, focus, create/close, server stop) belong in Copse.
- Live updates: poll `api snapshot`. The CLI has no subscribe command; a push channel would require implementing the socket protocol from `herdr api schema`.
- Outside a pane, the caller-context variables are absent; the snapshot's `focused_*` fields replace them.

## Citations

Quotes and claims map to sources as follows:

| Claim | Source |
| --- | --- |
| Herdr organizes terminals into workspaces, tabs, and panes, recognizes coding agents, exposes the session through the CLI | `herdr --skill`, opening paragraph |
| Agent status meanings (idle, done, blocked, unknown) and "CLI reads do not mark it seen" | `herdr --skill`, "Understand layout, panes, and agents" |
| "Herdr injects the caller's context into each managed pane" and the env vars | `herdr --skill`, "Use IDs and caller context" |
| "Do not inspect or control the focused Herdr session from outside Herdr" | `herdr --skill`, opening |
| IDs: workspace `w1`, tab `w1:t1`, pane `w1:p1`; closed IDs not reused; pane move re-qualifies | `herdr --skill`, "Use IDs and caller context" |
| Targets accept unique live agent names or pane IDs | `herdr --skill`, "Understand layout, panes, and agents" |
| `agent_status` enum values | `herdr api schema --json`, AgentStatus |
| AgentInfo and PaneInfo field shapes, nullable fields | `herdr api schema --json`, success_response/event `$defs` |
| `herdr worktree list` semantics and `not_git_worktree` error | `herdr worktree` help; live probe |
| `api snapshot`/`api schema` purpose ("Inspect socket API metadata and live runtime state") | `herdr --help`, Advanced commands |
| Event and subscription event kinds | `herdr api schema --json`, EventKind and SubscriptionEventKind enums |
| Error and exit-code contract | `herdr --skill`, "Safety and coordination rules" |
| Version and socket path | `herdr status`, live probe |
