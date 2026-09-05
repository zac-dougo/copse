# Copse

Terminal board for tracking work across worktrees.

Copse is a read-only board that shows one repository's worktrees, local issues, live Herdr agents, and Wayfinder maps. It runs in a Herdr pane and also works outside Herdr without live agent state.

## Install

Requires Rust 1.88+ (1.98 used in development) and Git.

```sh
# From this repo
cargo install --path . --force

# Or run without installing
cargo run

# Check
copse --version
copse --help
```

The binary is named `copse`.

## Usage in Herdr

Open a Herdr pane for your repository and run `copse`. Herdr organizes terminals into workspaces, tabs, and panes and exposes the session through `herdr api snapshot`. Copse polls that snapshot every 2 seconds and renders the forest.

```sh
# Inside a worktree (or any subdirectory of one)
copse
```

Keys:

- `1` Forest, `2` or `m` Map
- `Tab` cycle Wayfinder maps
- `↑` / `↓` move selection
- `←` / `→` collapse, expand, or move to parent/child in Forest
- `Enter` toggle the selected Issue's detail pane
- `PgUp` / `PgDn` scroll Map Issue details
- `r` refresh all sources now
- `?` toggle help
- `Esc` close detail or help
- `q` or `Ctrl-C` quit

Mouse is optional: click to select, wheel to scroll. No mouse-only actions. Mouse capture is disabled when Copse exits.

## Repository discovery

Copse uses Git as the source of truth, not Herdr.

- Started in a nested directory: resolves to the containing worktree.
- Started in a linked worktree: shows the same board and all worktrees. The primary worktree owns `.copse`.
- Missing `.copse`: starts with worktrees visible but no links. Starting does not create files.
- Outside a Git worktree: prints an error and exits without side effects.

## Worktree links

Issues live on GitHub. Copse only records which GitHub issue each worktree
is working on, as link files under the primary worktree's `.copse` directory:

```text
.copse/links/<uuid>.md
```

Each file is Markdown with strict TOML front matter delimited by `+++`:

```md
+++
id = "22222222-2222-4222-8222-222222222222"
issue = 42
worktree = "/home/zac/dev/projects/copse-worktrees/main"
+++
```

`issue` is the GitHub issue number. Unknown front matter keys are preserved.
Malformed records are not overwritten. Links from the old local tracker (UUID
`issue` values) are rejected; delete them and relink by issue number.

Branch names are derived from the linked worktree's Git metadata. Agent associations come from Herdr and are not persisted. GitHub Issues are read-only.

## Board

Forest layout is branch → Issue → Agent. The main branch is bold. Agent statuses are `idle`, `working`, `blocked`, `done`, and `unknown`. Display uses both text and a symbol:

- working blue `●`
- done green `✓`
- idle green `○`
- blocked red `!`
- unknown gray `·`

Map layout is a selected Wayfinder map grouped into Frontier, Blocked, Assigned, and Done sections. Frontier means an open Issue with no open blocker and no assignee. The Map view shows Wayfinder Issues only. It does not add Worktree or Agent context. The map with the most open children is selected first; `Tab` cycles through other maps. Copse reads GitHub maps with `gh issue list`. If GitHub is unavailable, the board keeps the last good data and the status bar marks GitHub stale.

Both views can open a detail pane for the selected Issue. Map details wrap the Issue body, include GitHub comments, and support `PgUp` / `PgDn` scrolling.

## Refresh

- Load all data on startup.
- Poll worktrees, links, and Herdr every 2 seconds.
- Poll GitHub Issues and Wayfinder every 30 seconds.
- `r` forces an immediate refresh.
- Keep the last good data when a source fails and show a stale indicator. If Herdr is unavailable, the board shows without live agent state.

## Development

```sh
cargo test
cargo run -- --help
cargo run -- --version
cargo fmt
cargo clippy -- -D warnings
```

Tests cover link parsing, repository discovery, Herdr snapshot parsing, forest building, and board interaction.

## Acceptance

- Starts in a nested directory and shows the containing worktree.
- Started in a linked worktree shows the same board and all worktrees.
- Missing `.copse` starts with worktrees only and does not create files.
- Outside a Git worktree prints an error and exits.
- Forest shows branches, linked Issues, and Agents with correct statuses and colors.
- Map shows GitHub Wayfinder Issues grouped by frontier state and keeps map order within each section.
- Issues tab shows every GitHub Issue grouped by frontier state.
- Keys and mouse work as listed, and refresh intervals behave as above.

## Stack

Rust, Ratatui with Crossterm, Tokio, pulldown-cmark, TOML, UUID, and Clap.
