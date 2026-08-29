# Terminal application runtime and UI-library options

Research for [issue 04](../.scratch/copse-v1/issues/04-research-terminal-app-options.md).
Findings only; no stack is chosen here. Sources are first-party documentation
(official project docs, official READMEs, official API docs) unless noted.

## Requirements under evaluation

Copse needs, per the issue and `CONTEXT.md`: keyboard-first navigation,
optional mouse input, correct handling of terminal resize, asynchronous
observation of the local Herdr session (external CLI, non-blocking), and
rendering of the local Markdown issue tracker. It is a read-only board app
rendered in a Herdr pane.

Five candidates were researched, each pairing a runtime with its primary TUI
library:

1. Rust + Ratatui (crossterm backend)
2. Go + Bubble Tea v2 (Charm ecosystem)
3. TypeScript/Node + Ink
4. Python + Textual
5. TypeScript/Node + `@earendil-works/pi-tui` (the library pi itself is built on)

## Candidate 1: Rust + Ratatui

**Runtime/toolchain.** Ratatui 0.30.2 requires Rust 1.88 or newer; installed
with `rustup`, then `cargo add ratatui crossterm`
(<https://ratatui.rs/installation/>). Crossterm 0.29 is the default backend;
termion, termwiz, and termina are alternatives, with the docs recommending
"Choose Crossterm for most tasks" (<https://ratatui.rs/concepts/backends/comparison/>).

**Keyboard.** Event handling is delegated to the backend. Crossterm's
`Event::Key(KeyEvent)` carries "a single key event with additional pressed
modifiers" (<https://docs.rs/crossterm/latest/crossterm/event/enum.Event.html>).
The official async example binds `j`/`k`/arrows for list navigation and `q`/`Esc`
to quit (<https://ratatui.rs/examples/apps/async-github/>). The event-handling
concept page documents centralized and message-passing key-dispatch patterns
(<https://ratatui.rs/concepts/event-handling/>).

**Mouse (optional).** Crossterm's `Event::Mouse(MouseEvent)` covers clicks,
scrolls, and movement; Ratatui documents "Mouse Capture" as a terminal mode the
app can enable (<https://ratatui.rs/concepts/backends/mouse-capture/>,
<https://docs.rs/crossterm/latest/crossterm/event/enum.Event.html>). A mouse
drawing example shipped in v0.29 demonstrates the events in use
(<https://ratatui.rs/highlights/v029/>).

**Resize.** Crossterm emits `Event::Resize(columns, rows)` ("a resize event with
new dimensions after resize"; note resize events can arrive in batches)
(<https://docs.rs/crossterm/latest/crossterm/event/enum.Event.html>). Ratatui's
constraint-based layout ("Think Flexbox, but for the terminal") adapts to any
terminal size (<https://ratatui.rs/>).

**Async Herdr observation.** First-party example: the async-github app runs a
tokio runtime, reads input from `crossterm::event::EventStream`, redraws on a
`tokio::time::interval` tick, and fetches data in background tasks sharing state
via `Arc<RwLock<_>>` (<https://ratatui.rs/examples/apps/async-github/>). The
same pattern supports polling an external CLI: spawn the command with
`tokio::process::Command` and merge its output into the select loop.

**Markdown.** No first-party Markdown widget; the website lists only third-party
widgets (<https://ratatui.rs/showcase/third-party-widgets/>). Rendering
tracker Markdown would need an ecosystem crate (e.g. a Markdown parser plus a
custom widget) or plain-text rendering.

## Candidate 2: Go + Bubble Tea v2

**Runtime/toolchain.** Bubble Tea v2's `go.mod` declares `go 1.25.0`
(<https://github.com/charmbracelet/bubbletea/blob/v2.0.9/go.mod>).

**Keyboard.** Model/Update receives `tea.KeyPressMsg` ("automatically sent to
the update function when keys are pressed"); the tutorial binds `up`/`k`,
`down`/`j`, `enter`, `space`, `q`, `ctrl+c`
(<https://github.com/charmbracelet/bubbletea/blob/v2.0.9/README.md>).
`KeyboardEnhancementsMsg` reports kitty-protocol capabilities
(<https://pkg.go.dev/charm.land/bubbletea/v2?tab=doc>).

**Mouse (optional).** "High-fidelity keyboard and mouse handling" is a headline
feature (<https://github.com/charmbracelet/bubbletea/blob/v2.0.9/README.md>).
v2 offers `MouseModeNone`, `MouseModeCellMotion`, `MouseModeAllMotion`
(click/release/wheel; SGR extended mode with X10 fallback) as view options, and
`MouseClickMsg`, `MouseMotionMsg`, `MouseReleaseMsg`, `MouseWheelMsg` messages
with an `OnMouse` view handler (<https://pkg.go.dev/charm.land/bubbletea/v2?tab=doc>).

**Resize.** `WindowSizeMsg{Width, Height}` is "sent to Update once initially and
then on every terminal resize" (<https://pkg.go.dev/charm.land/bubbletea/v2?tab=doc>).

**Async Herdr observation.** Concurrency is message-passing: `Cmd func() Msg`
("an IO operation that returns a message when it's complete... HTTP requests,
timers, saving and loading from disk"), `tea.Batch` for parallel commands,
`tea.Tick`/`tea.Every` for interval polling, and `tea.ExecProcess(*exec.Cmd, ...)`
for running external processes. This is the direct fit for observing a CLI
like Herdr (<https://pkg.go.dev/charm.land/bubbletea/v2?tab=doc>).

**Markdown.** Not built into Bubble Tea, but the same org ships Glamour
("stylesheet-based markdown rendering for your CLI apps", `glamour.Render(in, "dark")`)
(<https://github.com/charmbracelet/glamour/blob/main/README.md>). Component
library Bubbles provides inputs, viewports, spinners, etc.
(<https://github.com/charmbracelet/bubbles/blob/master/README.md>).

## Candidate 3: TypeScript/Node + Ink

**Runtime/toolchain.** Ink is "React for CLIs"; `npm install ink react`, runs on
Node (any modern LTS). In production use by Claude Code and Gemini CLI
(<https://github.com/vadimdemedes/ink/blob/master/readme.md>).

**Keyboard.** `useInput(handler, options)` delivers key events; the `key` object
exposes up/down/left/right, tab, pageUp/pageDown, etc.; kitty keyboard protocol
opt-in adds modifiers and press/repeat/release fidelity
(<https://github.com/vadimdemedes/ink/blob/master/readme.md>). Focus management
is first-party: `useFocus`/`useFocusManager` with automatic Tab/Shift+Tab
cycling (<https://github.com/vadimdemedes/ink/blob/master/readme.md>).

**Mouse.** No first-party mouse input API. The readme's only mouse mention is a
coordinate-conversion note in `measureElement`; there is no mouse capture or
mouse event hook (<https://github.com/vadimdemedes/ink/blob/master/readme.md>).
Mouse support would require third-party code.

**Resize.** `useWindowSize()` returns `{columns, rows}` and "re-renders the
component whenever the terminal is resized"
(<https://github.com/vadimdemedes/ink/blob/master/readme.md>).

**Async Herdr observation.** Native Node event loop: "an Ink app is a Node.js
process, so it stays alive only while there is active work in the event loop
(timers, pending promises, useInput listening on stdin, etc.)". React Suspense
works for async data fetching; child-process output rendering has an official
example (<https://github.com/vadimdemedes/ink/blob/master/readme.md>).

**Markdown.** Not built in. The readme lists `ink-markdown` ("Render syntax
highlighted Markdown") as a third-party community component
(<https://github.com/vadimdemedes/ink/blob/master/readme.md>).

## Candidate 4: Python + Textual

**Runtime/toolchain.** "Textual requires Python 3.9 or later"; install with
`pip install textual`; runs on Linux, macOS, Windows
(<https://textual.textualize.io/getting_started/>).

**Keyboard.** Key events arrive as `events.Key` with a normalized `key` string
(`"ctrl+p"`, `"shift+home"`, ...); bindings and input focus are framework
features (<https://textual.textualize.io/guide/input/>).

**Mouse (optional).** "Textual will send events in response to mouse movement
and mouse clicks" with coordinates relative to terminal or widget; the events
list includes `Click`, `MouseDown`, `MouseMove`, `MouseRelease`,
`MouseScrollUp/Down` (<https://textual.textualize.io/guide/input/>,
<https://textual.textualize.io/events/>).

**Resize.** A `Resize` event is part of the event set
(<https://textual.textualize.io/events/>).

**Async Herdr observation.** First-party Worker API (added in 0.18.0): run
`async def` coroutines in the background with `@work(exclusive=True)` or
`run_worker`, cancellation via `Worker.cancel`, and the guide explicitly covers
"reading from a subprocess or doing compute heavy work"
(<https://textual.textualize.io/guide/workers/>).

**Markdown.** First-party `Markdown` widget ("A widget to display a Markdown
document", focusable, since v0.11.0) plus `MarkdownViewer` with a table of
contents (<https://textual.textualize.io/widgets/markdown/>).

## Candidate 5: TypeScript/Node + @earendil-works/pi-tui

**Runtime/toolchain.** `@earendil-works/pi-tui` is the TUI library pi itself is
built on ("Terminal User Interface library with differential rendering";
maintained in the pi monorepo). pi runs on Node; pi bundles pi-tui for its
extensions (<https://www.npmjs.com/package/@earendil-works/pi-tui>,
<https://github.com/earendil-works/pi-mono/tree/main/packages/tui>,
pi docs `docs/packages.md`). Copse runs in a Herdr pane alongside pi, so this is
the library already proven in Copse's target environment. `ProcessTerminal`
reads `process.stdin/stdout`; `VirtualTerminal` (based on `@xterm/headless`)
supports testing (<https://www.npmjs.com/package/@earendil-works/pi-tui>).

**Keyboard.** Components receive keyboard input via `handleInput(data)`; the
`matchesKey`/`Key` helpers detect keys and support the kitty keyboard protocol;
`SelectList` is an "interactive selection list with keyboard navigation"
(<https://www.npmjs.com/package/@earendil-works/pi-tui>).

**Mouse (optional).** `TuiAltScreen` "supports mouse, trackpad, and keyboard
navigation": mouse-wheel scrolls the view under the pointer, clicking an OSC 8
hyperlink opens it, dragging selects and copies text (OSC 52)
(<https://www.npmjs.com/package/@earendil-works/pi-tui>).

**Resize.** The `Terminal` interface is `start(onInput, onResize)`; the
alternate-screen viewport tracks the terminal-height layout and "updates changed
viewport rows in place" (<https://www.npmjs.com/package/@earendil-works/pi-tui>).

**Async Herdr observation.** Node event loop; `CancellableLoader` couples an
`AbortSignal` to an async operation with Escape-key cancellation; `Loader` and
`ProcessTerminal` integrate with async flows
(<https://www.npmjs.com/package/@earendil-works/pi-tui>).

**Markdown.** First-party `Markdown` component: "Renders markdown with syntax
highlighting and theming support". It handles headings, bold, italic, code
blocks, lists, links, blockquotes; theme hooks per element; render caching;
depends on `marked`
(<https://www.npmjs.com/package/@earendil-works/pi-tui>).

## Cross-candidate comparison

| Criterion | Ratatui (Rust) | Bubble Tea (Go) | Ink (Node) | Textual (Python) | pi-tui (Node) |
| --- | --- | --- | --- | --- | --- |
| Keyboard-first | Backend events, j/k examples | KeyPressMsg, tutorial binds | useInput + focus mgmt | events.Key + bindings | handleInput/matchesKey, SelectList |
| Mouse (optional) | Crossterm Mouse events, capture mode | MouseMode view options + mouse msgs | **No first-party API** | Click/MouseDown/Move/Release/Scroll events | Wheel scroll, hyperlink click, drag-select |
| Resize | Event::Resize(rows, cols) | WindowSizeMsg on every resize | useWindowSize re-render | Resize event | onResize callback, viewport tracking |
| Async external observation | tokio + EventStream + interval; Arc<RwLock> shared state | Cmd/Batch/Tick/Every + ExecProcess | Node event loop, Suspense, child-process example | Worker API, subprocess coverage | Node event loop, AbortSignal loaders |
| Markdown rendering | Not built in (ecosystem crates) | Not built in (Glamour from same org) | Not built in (ink-markdown third-party) | Built-in Markdown + MarkdownViewer | Built-in Markdown component |
| Toolchain to install | Rust 1.88+ (not installed here) | Go 1.25.0 (not installed here) | Node present (v26.7.0) | Python present (3.14.4) | Node present (v26.7.0) |

## Environment notes (this machine)

- Node v26.7.0 is installed (nvm); Python 3.14.4 is installed.
- Go and Rust toolchains are **not** installed.
- pi (Node/TypeScript TUI) runs in this environment and is already a Herdr
  session participant; pi-tui is its bundled TUI library.

## Sources

- Ratatui: <https://ratatui.rs/> and subpages (installation, backends, mouse
  capture, event handling, async-github example, v0.29/v0.30 highlights);
  <https://docs.rs/crossterm/latest/crossterm/event/enum.Event.html>.
- Bubble Tea v2: <https://github.com/charmbracelet/bubbletea/blob/v2.0.9/README.md>,
  <https://pkg.go.dev/charm.land/bubbletea/v2?tab=doc>,
  <https://github.com/charmbracelet/bubbletea/blob/v2.0.9/go.mod>,
  <https://github.com/charmbracelet/glamour/blob/main/README.md>,
  <https://github.com/charmbracelet/bubbles/blob/master/README.md>.
- Ink: <https://github.com/vadimdemedes/ink/blob/master/readme.md>.
- Textual: <https://textual.textualize.io/> (getting started, guide/input,
  guide/workers, events, widgets/markdown).
- pi-tui: <https://www.npmjs.com/package/@earendil-works/pi-tui>,
  <https://github.com/earendil-works/pi-mono/tree/main/packages/tui>,
  pi docs (`README.md`, `docs/packages.md`, `docs/extensions.md`).
