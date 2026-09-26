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
`open_editor` tool.

The editor shows each connected client at the top of the window, for
example "Claude Code · MCP". Click it to see the clients.

## Use the CLI

The CLI calls the open editor and writes JSON to stdout. Errors go to stderr
as `{"error": {...}}`, and the exit code is 1.

```sh
sliderino instances                     # list_instances
sliderino open                          # open_editor
sliderino info                          # get_basic_info
sliderino screenshot -o slide.png       # get_screenshot
sliderino apply ops.json --label "Add title"   # apply_operations
sliderino undo                          # undo
sliderino redo                          # redo
sliderino tools                         # the tools and their arguments
sliderino call get_slide '{"slide": 2}' # any tool
```

`sliderino apply -` and `sliderino call TOOL -` read from stdin.

## Select an instance

Each editor process is one instance. When one instance is open, all tools
use it. When more instances are open, give the process id:

- MCP: the `instance` argument of the tool.
- CLI: `--instance PID` or the `SLIDERINO_INSTANCE` environment variable.

If you do not give the process id, the tool fails with `several_instances`.
The error data lists the instances.

## Tools

| Tool | Function |
| --- | --- |
| `list_instances` | List the open editors. |
| `open_editor` | Start an editor and wait until it accepts calls. |
| `get_basic_info` | Get the revision, the slides, the fonts and the undo state. |
| `get_selection` | Get the slide, the element and the text that the person selected. |
| `get_slide` | Get the elements of a slide in paint order. |
| `get_elements` | Get elements by id, with the text layout and the overflow. |
| `find_elements` | Find text elements that contain a string. |
| `get_screenshot` | Render a slide to PNG. The default scale is 0.5 (800 × 450). |
| `get_diagnostics` | List text that overflows its box and characters that the font does not have. |
| `list_fonts` | List the embedded faces and the families that you can embed. |
| `apply_operations` | Apply a list of operations as one undo step. |
| `undo` | Undo the latest step, from a person or from an agent. |
| `redo` | Redo the latest undone step. |

Tools that do not get a `slide` argument use the slide that the editor
shows.

## Operations

`apply_operations` gets a list of operations. The format is the same as the
format of the debug scenes (see `debug-scenes.md`):

```json
[
  {"op": "add_slide", "slide": {"id": "$s"}},
  {"op": "add_element", "slide": "$s", "element": {
    "id": "$title", "frame": {"x": 120, "y": 80, "width": 900},
    "text": {"content": "Activation grew faster than signups", "sizing": "auto_height",
             "style": {"font": {"family": "Inter", "weight": 600}, "size": 72}}}},
  {"op": "set_text_style", "id": "$title", "patch": {"color": "1F4BFF"}}
]
```

Rules:

- All operations of a call apply, or no operation applies. If an operation
  fails, the error data gives its index (from 0) in `op`.
- One call is one undo step. The `label` argument names the step.
- Do not give the ids of new slides and elements. The editor gives the next
  free id.
- To use a new id in a later operation of the same call, write a reference
  such as `"$title"` as the id. The result gives the id of each reference in
  `refs`.
- The editor embeds each font face that an operation uses. It gets the face
  from the fonts of Sliderino or from the fonts of the system. If the
  license of the face does not permit embedding, the editor embeds it and
  the result has a warning in `warnings`.

## Detect changes by other clients

Each change increments the revision of the document. All tools that read or
change the document give the `revision`.

To make sure that you change the version that you read, give
`base_revision` to `apply_operations`, `undo` or `redo`. If the document
changed after that revision, the tool fails with `stale_revision` and changes
nothing. The error data gives the current `revision`. Read the document
again, then send the change again.

## Follow the agent

By default, agent changes do not change the slide or the selection that the
person sees. To show each agent change, click the agent status at the top of
the window and set **Follow agent**. The editor then shows the slide of each
change and selects the changed element.

## Errors

| Code | Cause |
| --- | --- |
| `no_instance` | No editor is open. |
| `several_instances` | More than one editor is open and the call gives no `instance`. |
| `unknown_instance` | No editor has the given process id. |
| `invalid_arguments` | The arguments do not match the schema of the tool. |
| `unknown_slide`, `unknown_element` | The id does not exist. |
| `operation_failed` | An operation cannot apply. `data.op` gives its index. |
| `stale_revision` | The document changed after `base_revision`. |
| `connection_failed` | The editor closed the connection. |

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
