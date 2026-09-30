# Agent API

External agents read and change the presentation that is open in the
Sliderino editor. They use the Model Context Protocol (MCP) server or the
command-line interface (CLI). Both give the same tools.

Each change appears in the editor immediately. The person and the agents
share one undo history.

## Connect an MCP client

Add the server to the client. For Claude Code:

```sh
claude mcp add sliderino -- sliderino mcp
```

The client starts `sliderino mcp` and stops it. The editor does not have to
be open when the client starts the server. If no editor is open, call the
`open_editor` tool. It opens an editor with a new presentation.

A window of Sliderino shows the Home screen or the editor. On the Home
screen, only `new_presentation` and `open_presentation` work. The other
tools of the instance fail with `no_presentation`.

The editor shows each connected client at the top of the window, for
example "Claude Code · MCP". Click it to see the clients.

## Give the guide to the agent

The text from "Work with a presentation" to "Errors" on this page is also a
skill for agents, `SKILL.md`. Sliderino gives it in two ways:

- MCP: the resource `skill://sliderino/SKILL.md`.
- CLI: `sliderino skill` writes it to stdout.

To install it as a skill of Claude Code:

```sh
mkdir -p ~/.claude/skills/sliderino
sliderino skill > ~/.claude/skills/sliderino/SKILL.md
```

{{#include ../src/api/SKILL.md:guide}}

## Follow the agent

By default, agent changes do not change the slide or the selection that the
person sees. To show each agent change, click the agent status at the top of
the window and set **Follow agent**. The editor then shows the slide of each
change and selects the changed element.

## How it works

Each editor opens a Unix socket and writes a description file:

- `$XDG_RUNTIME_DIR/sliderino/<pid>.sock`
- `$XDG_RUNTIME_DIR/sliderino/<pid>.json`

If `XDG_RUNTIME_DIR` is not set, the directory is
`$TMPDIR/sliderino-<uid>`. Only the user can read the directory and the
socket. Clients remove the files of editors that stopped.

`sliderino mcp` and the CLI send one JSON message on each line of the
socket. Start the editor with `sliderino --no-api` to not open the socket.

The API works on Linux and macOS only.
