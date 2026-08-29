#!/usr/bin/env python3
"""THROWAWAY PROTOTYPE: three terminal board visualizations for Copse.

Run with: python prototype/board_visualization.py
Keys: left/right or h/l switch views, 1/2/3 choose a view, s toggles sample
state, q quits. This prototype chooses a layout, not a production UI toolkit.
"""

import curses
import json

STATE = {
    "repository": "copse",
    "worktrees": [
        {
            "branch": "main",
            "path": "/home/zac/dev/projects/copse",
            "issue": None,
            "agent": None,
            "agent_state": None,
        },
        {
            "branch": "prototype/board-visualization",
            "path": "/home/zac/dev/projects/copse-prototype-board-visualization",
            "issue": "Prototype board visualization",
            "agent": "zac",
            "agent_state": "working",
        },
        {
            "branch": "research/herdr-observation",
            "path": "/home/zac/dev/projects/copse-research-herdr-observation",
            "issue": "Research Herdr observation",
            "agent": "scout",
            "agent_state": "done",
        },
        {
            "branch": "feature/tracker-links",
            "path": "/home/zac/dev/projects/copse-tracker-links",
            "issue": "Define local tracker schema",
            "agent": "moth",
            "agent_state": "blocked",
        },
    ],
    "wayfinder": {
        "map": "Copse v1",
        "open": 6,
        "blocked": 2,
        "resolved": 2,
    },
}

VIEWS = [
    ("Forest", "Nested worktree ownership"),
    ("Lanes", "Status-first worktree scan"),
    ("Map", "Issue dependencies with worktree context"),
]


def add(stdscr, row, col, text, attr=0):
    height, width = stdscr.getmaxyx()
    if 0 <= row < height and col < width:
        stdscr.addnstr(row, max(col, 0), text, max(0, width - col - 1), attr)


def heading(stdscr, view):
    add(stdscr, 0, 2, f"COPSE / {STATE['repository']}   {view[0].upper()}", curses.A_BOLD)
    add(stdscr, 1, 2, view[1], curses.A_DIM)
    add(stdscr, 2, 0, "─" * 200, curses.A_DIM)


def status_badge(state):
    return {
        "working": "● working",
        "done": "✓ done",
        "blocked": "! blocked",
        None: "· no agent",
    }[state]


def forest(stdscr):
    add(stdscr, 4, 2, "repository", curses.A_BOLD)
    for index, tree in enumerate(STATE["worktrees"]):
        row = 6 + index * 4
        joint = "└─" if index == len(STATE["worktrees"]) - 1 else "├─"
        add(stdscr, row, 2, f"{joint} {tree['branch']}", curses.A_BOLD)
        issue = tree["issue"] or "No linked issue"
        add(stdscr, row + 1, 6, f"issue  {issue}")
        add(stdscr, row + 2, 6, f"agent  {tree['agent'] or '—'}  {status_badge(tree['agent_state'])}")
    add(stdscr, 23, 2, "Read this as: branch → issue → agent. Empty links are visible gaps.", curses.A_DIM)


def lanes(stdscr):
    columns = [
        ("Working", [STATE["worktrees"][1]]),
        ("Blocked", [STATE["worktrees"][3]]),
        ("Done", [STATE["worktrees"][2]]),
        ("Unlinked", [STATE["worktrees"][0]]),
    ]
    height, width = stdscr.getmaxyx()
    column_width = max(18, (width - 8) // len(columns))
    for index, (title, trees) in enumerate(columns):
        col = 2 + index * column_width
        add(stdscr, 4, col, title.upper(), curses.A_BOLD | curses.A_UNDERLINE)
        for offset, tree in enumerate(trees):
            row = 6 + offset * 7
            add(stdscr, row, col, tree["branch"], curses.A_BOLD)
            add(stdscr, row + 1, col, "─" * (column_width - 3), curses.A_DIM)
            add(stdscr, row + 2, col, tree["issue"] or "No linked issue")
            add(stdscr, row + 3, col, status_badge(tree["agent_state"]))
            add(stdscr, row + 4, col, tree["agent"] or "No agent", curses.A_DIM)
    add(stdscr, 23, 2, "Read this as: what needs attention now. Branch structure comes second.", curses.A_DIM)


def dependency_map(stdscr):
    add(stdscr, 4, 2, "WORKTREES", curses.A_BOLD)
    for index, tree in enumerate(STATE["worktrees"]):
        marker = "●" if tree["agent_state"] == "working" else "!" if tree["agent_state"] == "blocked" else "·"
        add(stdscr, 6 + index * 2, 2, f"{marker} {tree['branch']}")

    add(stdscr, 4, 38, "WAYFINDER / COPSE V1", curses.A_BOLD)
    add(stdscr, 6, 38, "[Research Herdr observation] ──┐", curses.A_DIM)
    add(stdscr, 7, 38, "[Research terminal app options] ─┼─> [Choose terminal app stack]")
    add(stdscr, 8, 38, "[Prototype board visualization] ──┘              │", curses.A_BOLD)
    add(stdscr, 9, 38, "                                              ┌──┴──┐")
    add(stdscr, 10, 38, "                                   [Define board interaction]")
    add(stdscr, 12, 38, "[Decide issue write boundary] ─> [Define local tracker schema] ─┘")
    add(stdscr, 16, 38, f"open {STATE['wayfinder']['open']}   blocked {STATE['wayfinder']['blocked']}   resolved {STATE['wayfinder']['resolved']}")
    add(stdscr, 23, 2, "Read this as: which decision unlocks the next move. Worktrees stay visible as live context.", curses.A_DIM)


def state_overlay(stdscr):
    encoded = json.dumps(STATE, indent=2).splitlines()
    height, width = stdscr.getmaxyx()
    start = 4
    add(stdscr, start, max(2, width // 2), "SAMPLE STATE", curses.A_BOLD | curses.A_REVERSE)
    for offset, line in enumerate(encoded[: height - start - 4]):
        add(stdscr, start + 1 + offset, max(2, width // 2), line, curses.A_DIM)


def bottom_bar(stdscr, view_index, show_state):
    height, width = stdscr.getmaxyx()
    keys = "←/h previous   1 Forest   2 Lanes   3 Map   →/l next   s state   q quit"
    label = f" {view_index + 1}/3 {VIEWS[view_index][0]} "
    add(stdscr, height - 2, max(0, (width - len(keys)) // 2), keys, curses.A_REVERSE)
    add(stdscr, height - 1, max(0, (width - len(label)) // 2), label + ("  STATE ON" if show_state else ""), curses.A_BOLD)


def draw(stdscr, view_index, show_state):
    stdscr.erase()
    heading(stdscr, VIEWS[view_index])
    [forest, lanes, dependency_map][view_index](stdscr)
    if show_state:
        state_overlay(stdscr)
    bottom_bar(stdscr, view_index, show_state)
    stdscr.refresh()


def app(stdscr):
    curses.curs_set(0)
    stdscr.keypad(True)
    view_index = 0
    show_state = False
    while True:
        draw(stdscr, view_index, show_state)
        key = stdscr.getch()
        if key in (ord("q"), 27):
            return
        if key in (curses.KEY_LEFT, ord("h")):
            view_index = (view_index - 1) % len(VIEWS)
        elif key in (curses.KEY_RIGHT, ord("l")):
            view_index = (view_index + 1) % len(VIEWS)
        elif key in (ord("1"), ord("2"), ord("3")):
            view_index = int(chr(key)) - 1
        elif key == ord("s"):
            show_state = not show_state


if __name__ == "__main__":
    curses.wrapper(app)
