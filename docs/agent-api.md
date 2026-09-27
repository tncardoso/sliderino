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

## Use the CLI

The CLI calls the open editor and writes JSON to stdout. Errors go to stderr
as `{"error": {...}}`, and the exit code is 1.

```sh
sliderino instances                     # list_instances
sliderino open                          # open_editor
sliderino info                          # get_basic_info
sliderino new                           # new_presentation
sliderino open-file deck.sldr           # open_presentation
sliderino save deck.sldr                # save_presentation
sliderino screenshot -o slide.png       # get_screenshot
sliderino shader-video 4 -o shader.mp4  # render_shader_video
sliderino apply ops.json --label "Add title"   # apply_operations
sliderino undo                          # undo
sliderino redo                          # redo
sliderino tools                         # the tools and their arguments
sliderino call get_slide '{"slide": 2}' # any tool
```

`sliderino apply -` and `sliderino call TOOL -` read from stdin.

`sliderino deck.sldr` opens the file in a new editor window. `sliderino`
without a file opens the Home screen.

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
| `new_presentation` | Show a new presentation in the editor. |
| `open_presentation` | Open a `.sldr` file in the editor. |
| `save_presentation` | Save the presentation to a `.sldr` file. |
| `get_basic_info` | Get the file, the unsaved state, the revision, the slides, the fonts, the images, the videos and the undo state. |
| `get_selection` | Get the slide, the list of slides, the list of elements and the text that the person selected. |
| `get_slide` | Get the elements of a slide in paint order. |
| `get_elements` | Get elements by id, with the text layout and the overflow. |
| `find_elements` | Find text elements that contain a string. |
| `get_screenshot` | Render a slide to PNG. The default scale is 0.5 (800 × 450). Videos show their first frame. Shaders show their frame at `time` (default 0). |
| `get_diagnostics` | List text that overflows its box, characters that the font does not have, shaders that do not compile, and videos and shaders that cannot play on this computer. |
| `render_shader_video` | Render the shader fill of an element to an MP4 file. |
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
- `duplicate_slide` copies a slide and its elements. The copy and its
  elements get new ids. The copy goes after the slide, or to `index` if you
  give it. To use the id of the copy in a later operation, give a reference
  in `ref`:
  `{"op": "duplicate_slide", "id": 3, "ref": "$copy"}`.
- `remove_slide` fails on the last slide of the presentation. Add a slide
  before you remove it.
- The editor embeds each font face that an operation uses. It gets the face
  from the fonts of Sliderino or from the fonts of the system. If the
  license of the face does not permit embedding, the editor embeds it and
  the result has a warning in `warnings`.

## Groups and layers

The elements of a slide are a tree. A group holds its children in paint
order: the last child is on top. `get_slide` gives the tree, with the
children of each group in `group.children`.

- All frames are in slide units, also the frames of children. The rotation
  of a child is its angle on the slide.
- The frame of a group is the box around the frames of its children, in the
  axes of the rotation of the group. The editor calculates it.
- `set_frame` on a group with the same size and rotation moves the group and
  its children.
- `set_frame` on a group with a different size scales the positions and the
  boxes of the children. The font sizes do not change. A text with
  `auto_width` sizing keeps its size and only moves.
- `set_frame` on a group with a different rotation turns the group and its
  children around the center of the group, to the angle you give.
- A new group has the rotation 0. In `add_element`, the rotation of a group
  only sets the axes of its box. The children do not turn.
- `group` puts elements of one slide into a new group. The group goes to the
  position of the topmost element.
- `ungroup` puts the children of a group in its position and removes the
  group.
- `move_element` moves an element into a group or out of a group. The
  element keeps its frame.
- `group` and `ungroup` read the document after the earlier operations of
  the call. In a `batch`, they cannot use elements that the same batch adds.

Use `set_layer` to set the name, the visibility, the lock and the opacity
of an element:

```json
{"op": "set_layer", "id": 12, "patch": {"name": "Header", "locked": true}}
```

- `opacity` is a number from 0 to 1. The default is 1. It applies to all
  elements: texts, shapes and groups. The opacity of a group multiplies the
  opacity of each child.

- A hidden element and the children of a hidden group do not show in the
  editor, in screenshots or in diagnostics.
- A locked element, and each element in a locked group, accepts only
  `set_layer`. Other operations fail with `operation_failed`. To change the
  element, set `"locked": false` first.
- The person can move a group that holds a locked element. The locked
  element moves with the group.

## Shapes

A slide can have rectangles, ellipses and lines. Add them with
`add_element`:

```json
[
  {"op": "add_element", "slide": 1, "element": {"id": "$card",
    "frame": {"x": 100, "y": 100, "width": 400, "height": 240},
    "rectangle": {"corner_radius": 16,
      "fill": {"linear_gradient": {"angle": 90, "stops": [
        {"position": 0, "color": "1F4BFF"}, {"position": 1, "color": "7FD4FF"}]}},
      "stroke": {"color": "111111", "width": 2, "dash": "dashed"}}}},
  {"op": "add_element", "slide": 1, "element": {
    "frame": {"x": 600, "y": 120, "width": 200, "height": 200},
    "ellipse": {"fill": "none", "stroke": {"width": 6}}}},
  {"op": "add_element", "slide": 1, "element": {
    "line": {"from": {"x": 100, "y": 500}, "to": {"x": 400, "y": 600}, "end": "triangle"}}}
]
```

- An ellipse touches the four sides of its frame. A square frame gives a
  circle.
- A new rectangle or ellipse has a light gray fill and no stroke.
- The frame of a line has a height of 0. The line goes from the left end to
  the right end of the frame. The rotation of the frame is the angle of the
  line. If you give a frame with a height, the line is the horizontal center
  axis of that frame.
- To place a line by its ends, give `from` and `to` instead of a frame. To
  move the ends later, use `set_line_points`. `get_elements` gives the ends
  of a line in `points`.
- When a group changes size, the ends of each line in the group move with
  the group. The stroke width and the corner radius do not change.

Fill (rectangles and ellipses):

| Fill | Fields |
| --- | --- |
| `"none"` | |
| `{"solid": {..}}` | `color`, `opacity` (optional, 0 to 1) |
| `{"linear_gradient": {..}}` | `angle` (degrees, clockwise; 0 goes from left to right), `stops` |
| `{"radial_gradient": {..}}` | `center` (optional, `{"x": 0.5, "y": 0.5}`), `radius` (optional, `{"x": 0.5, "y": 0.5}`), `stops` |

- A gradient has 2 to 10 `stops`. A stop is `{"position": 0.5, "color":
  "FFFFFF", "opacity": 1}`. The positions go from 0 to 1, in increasing
  order.
- A gradient turns with its shape. The angle of a linear gradient is in the
  axes of the shape.
- The `center` and the `radius` of a radial gradient are fractions of the
  frame. The radius 0.5 touches the sides of the frame.

Stroke (all shapes):

| Field | Value |
| --- | --- |
| `color` | Hex color. The default is `"111111"`. |
| `opacity` | 0 to 1. The default is 1. |
| `width` | Slide units, more than 0. The default is 4. |
| `dash` | `solid`, `dashed` or `dotted`. The default is `solid`. |

- The stroke is on the center of the outline.
- A rectangle or an ellipse without `stroke` has no stroke. A line always
  has a stroke.

Other fields:

- `corner_radius` (rectangles): the radius of all corners, in slide units.
  A radius larger than half of the shorter side shows as half of the shorter
  side.
- `start` and `end` (lines): the arrowhead at each end. Use `none`,
  `triangle`, `arrow`, `diamond` or `circle`. To set the size, use
  `{"kind": "triangle", "size": "large"}`. The sizes are `small`, `medium`
  (the default) and `large`: 2, 3 and 5 times the stroke width.

Use `set_shape_style` to change the style. Give only the fields to change:

```json
{"op": "set_shape_style", "id": 12, "patch": {"stroke": {"width": 8}, "end": "arrow"}}
```

- In `stroke`, give only the stroke fields to change. `"stroke": null`
  removes the stroke of a rectangle or an ellipse.
- A field that the shape does not have makes the operation fail. For
  example, a line has no `fill`.

## Tables

A table is a grid of text cells. Add it with `add_element`:

```json
[
  {"op": "add_element", "slide": 1, "element": {"id": "$sales",
    "frame": {"x": 100, "y": 100},
    "table": {"cells": [["Fruit", "Qty"], ["Apples", "3"], ["Pears", "12"]],
      "text": {"size": 28}, "fill": {"solid": {"color": "F4F6FA"}}}}},
  {"op": "set_cell_style", "id": "$sales", "rows": {"start": 0, "end": 1},
    "fill": {"solid": {"color": "1F4BFF"}}, "text": {"color": "FFFFFF"}},
  {"op": "set_borders", "id": "$sales", "rows": {"start": 0, "end": 1},
    "sides": "bottom", "stroke": {"width": 3, "opacity": 1}}
]
```

Size:

- The text in a cell does not wrap. `\n` starts a new line.
- Each column is as wide as its widest cell, and each row is as tall as its
  tallest cell, plus `padding` (default 12) on each side. A column is at
  least 2 font sizes wide.
- `width` and `height` are `"auto"` (the default) or `{"fixed": 400}`. A
  fixed axis is at least that size: the extra space goes to all columns (or
  rows) in equal parts. The table is larger when its content needs more.
- `set_frame` with a new width or height fixes that axis. A size that is
  not larger than the content sets the axis back to `"auto"`.
  `set_table_sizing` sets `width` and `height` directly.
- When a group changes size, a table in it gets the new size as a fixed
  size. The font size does not change.

Cells:

- A cell is a text, or an object: `{"content": "3", "fill": {..}, "text":
  {"align": "right"}, "span": {"rows": 1, "columns": 2}}`.
- `text` of the table is a style patch over the table text style: Inter 24,
  `vertical_align` `middle`. `text` of a cell is a style patch over the
  table text style.
- `fill` of the table is the fill of each cell. The default is `"none"`. The
  `fill` of a cell replaces it. Cells cannot show videos or shaders.
- Rows and columns count from 0. A range is `{"start": 0, "end": 2}`; `end`
  is not in the range. `rows` or `columns` that you do not give mean all
  rows or all columns.

Merged cells:

- The top-left cell of a merge (the anchor) has a `span` and holds the text.
  The other cells of the merge are empty and are not drawn.
- `merge_cells` merges a range. The texts that are not empty become the
  lines of the anchor. `split_cell` splits the merge that holds a cell. The
  text stays in the anchor.
- A range always grows to hold complete merges.
- Inserted rows or columns in a merge make the merge larger. Removed rows or
  columns make it smaller. An operation that cuts a merge in two fails
  (`move_rows` and `move_columns`).

Borders:

- `stroke` of the table is the stroke of each edge. The default is
  `{"color": "111111", "opacity": 0.4, "width": 2}`. `"stroke": null` draws
  no grid.
- `set_borders` sets the edges of a range that `sides` gives: `all`,
  `outside`, `inside`, `inside_horizontal`, `inside_vertical`, `top`,
  `bottom`, `left` or `right`. `stroke` is a stroke (only the fields that
  change from the table stroke), `null` (no line) or `"inherit"` (the table
  stroke again).
- Two neighbor cells share one edge.

Operations:

| `op` | Fields |
| --- | --- |
| `set_cell_text` | `id`, `row`, `column`, `text`, `range` (optional; bytes of the text to replace) |
| `set_cells` | `id`, `row` (default 0), `column` (default 0), `values` (rows of texts). The table grows when the values do not fit. |
| `insert_rows`, `insert_columns` | `id`, `index` (the new rows go before it), `count` (default 1). They copy the style of the row or column before them. |
| `remove_rows`, `remove_columns` | `id`, `index`, `count` (default 1). A table keeps at least one row and one column. |
| `move_rows`, `move_columns` | `id`, `index`, `count` (default 1), `to` (the rows go before this row) |
| `merge_cells` | `id`, `rows`, `columns` |
| `split_cell` | `id`, `row`, `column` |
| `set_table_style` | `id`, `text`, `fill`, `stroke`, `padding` (all optional) |
| `set_cell_style` | `id`, `rows`, `columns`, `fill` (`null`: the table fill), `text`, `reset_text` (removes the text overrides first) |
| `set_borders` | `id`, `rows`, `columns`, `sides`, `stroke` |
| `set_table_sizing` | `id`, `width`, `height` |

- The operations in one `batch` see the changes of the operations before
  them.
- `get_elements` adds `layout` to a table: `columns` and `rows` (their
  sizes) and `missing_glyphs`.
- `find_elements` finds the text of table cells. A match in a table adds
  `cell` (`{"row": 1, "column": 0}`).

## Fonts

A presentation holds its fonts. The editor embeds an installed face when an
operation uses it. To use a font that is not installed, add its file with
`add_font`:

```json
[
  {"op": "add_font", "path": "/home/me/fonts/Brand-Regular.ttf"},
  {"op": "add_font", "path": "/home/me/fonts/Brand-Bold.ttf"}
]
```

- Sliderino supports TTF, OTF and TTC files. It does not support WOFF and
  WOFF2 files. Convert them to TTF or OTF first.
- Give `path` or `data`. The CLI and the MCP server read the file of `path`
  and send its bytes to the editor. A relative path starts from the folder
  where the client runs. `data` is the file in base64.
- `add_font` adds all faces of the file. To add only one face of a TTC
  file, also give `face`.
- The editor does not examine the license of a font file. You must have the
  license to use the font.
- If other fonts already use the family name of the file, the editor gives
  the family a new name, for example `Brand (2)`. The result has a warning
  in `warnings`. A later file of the same family goes into `Brand (2)` if
  that family does not already have the same weight and slant.
- The result gives the faces of the file in `fonts`, with their family
  names in the presentation. Use these names in the text style.
- If the presentation already holds the same face with the same bytes,
  `add_font` does not add it again.
- A variable font adds one face: its default instance. The result has a
  warning in `warnings`.
- `remove_font` with `face` removes a face that no text uses.
  `remove_font` with `family` removes all faces of the family. The texts
  that use the family change to the nearest face of Inter. You cannot
  remove Inter while texts use it.

## Images

A presentation holds its images. Add an image with `add_image`, then use it
as the fill of a rectangle or an ellipse:

```json
[
  {"op": "add_image", "id": "$logo", "path": "/home/me/logo.png"},
  {"op": "add_element", "slide": 1, "element": {
    "frame": {"x": 100, "y": 100, "width": 320, "height": 180},
    "rectangle": {"corner_radius": 12, "fill": {"image": {"id": "$logo", "fit": "cover"}}}}}
]
```

- Sliderino supports PNG and JPEG images. An image can have at most 16384
  pixels on a side and 64 MiB.
- Give `path` or `data`. The CLI and the MCP server read the file of `path`
  and send its bytes to the editor. A relative path starts from the folder
  where the client runs. `data` is the file in base64.
- If the presentation already holds the same bytes, `add_image` does not add
  them again. The reference names the image that is already there.
- An image fill is `{"image": {"id", "fit", "opacity"}}`. `fit` is `cover`
  (the default: the image covers the shape and the parts outside are cut
  off), `contain` (all of the image shows in the shape) or `stretch` (the
  image fills the frame and its proportions change).
- A JPEG keeps its EXIF orientation. `width` and `height` in
  `get_basic_info` are the size of the upright image. Use them to give the
  frame the proportions of the image.
- `remove_image` removes an image that no fill uses. An image that a fill
  uses stays in the presentation after you delete the element, so that undo
  can restore the element.

## Videos

A presentation holds its videos. Add a video with `add_video`, then use it
as the fill of a rectangle or an ellipse:

```json
[
  {"op": "add_video", "id": "$clip", "path": "/home/me/demo.webm"},
  {"op": "add_element", "slide": 1, "element": {
    "frame": {"x": 160, "y": 90, "width": 1280, "height": 720},
    "rectangle": {"fill": {"video": {"id": "$clip", "start": "on_click", "loop": false}}}}}
]
```

- Sliderino keeps each video as an MP4 file with H.264 video and AAC audio.
  PowerPoint and web browsers can play this format. If the file has an
  other format, the editor changes it to MP4 before it applies the
  operations. This can take a long time.
- A video can have at most 512 MiB.
- Give `path` or `data`, as for `add_image`.
- If the presentation already holds the same bytes, `add_video` does not add
  them again.
- A video fill is `{"video": {"id", "fit", "opacity", "start", "loop",
  "muted"}}`. `fit` is the same as for images. `start` is `auto` (the
  default) or `on_click`. `loop` is `true` by default. `muted` is `false` by
  default.
- A presentation plays the videos. A screenshot and a PDF show the first
  frame of the video.
- `remove_video` removes a video that no fill uses.
- Sliderino uses GStreamer to read, change and play videos. If a GStreamer
  plugin is missing, `add_video` fails and `get_diagnostics` gives a
  `media_error`.

## Shaders

A shader fill draws a fragment shader in the shape. Write the shader in GLSL
as on Shadertoy:

```json
{"op": "set_shape_style", "id": 4, "patch": {"fill": {"shader": {
  "source": "void mainImage(out vec4 fragColor, in vec2 fragCoord)\n{\n    vec2 uv = fragCoord / iResolution.xy;\n    fragColor = vec4(uv, 0.5 + 0.5 * sin(iTime), 1.0);\n}\n",
  "duration": 6}}}}
```

- The shader must have `void mainImage(out vec4 fragColor, in vec2
  fragCoord)`. `fragCoord` is in pixels from the bottom-left corner of the
  shape box.
- The shader can use `iResolution`, `iTime`, `iTimeDelta`, `iFrame`,
  `iFrameRate`, `iMouse` (always 0), `iDate` (always 0), `iChannel0` and
  `iChannelResolution`. The shader has one pass.
- `channel0` gives an image of the presentation (an id or a `"$name"` of
  `add_image`) for `iChannel0`. As on Shadertoy, `texture(iChannel0,
  fragCoord / iResolution.xy)` shows the image upright.
- The alpha of `fragColor` has no effect: the shader is opaque. Use
  `opacity` to show what is below.
- Without `source`, the fill gets the default shader of Shadertoy.
- `duration` is from 1 to 60 seconds (default 10). A shader with `loop`
  starts again after `duration`. A shader without `loop` stops at
  `duration`. `start` is the same as for videos.
- PPTX gets the shader as a video of `duration` seconds. Use
  `render_shader_video` to make this video and examine it. A screenshot and
  a PDF show the shader at time 0; `get_screenshot` with `time` shows it at
  an other time.
- A shader that does not compile shows a gray box. `get_diagnostics` gives
  the problem as `shader_error` with the `line` of the source and the
  `message`.

## Files

A `.sldr` file holds the presentation with its fonts, images and videos. It
is a zip archive:

- `presentation.json` holds the format number, the slide size, the slides,
  the next ids and the list of the embedded files.
- `fonts/`, `images/` and `videos/` hold the embedded files as they were
  added.

Use these tools:

- `save_presentation {path?}` writes the file. Without `path`, it writes the
  file of the presentation. If the presentation has no file, the tool fails
  with `no_file`. With `path`, it replaces the file if it exists, and the
  path becomes the file of the presentation. It adds `.sldr` when the path
  has no extension.
- `open_presentation {path, discard?}` shows the file in the editor.
- `new_presentation {discard?}` shows a new presentation in the editor.

A relative `path` starts at the working folder of the client.

`open_presentation` and `new_presentation` fail with `unsaved_changes` when
the open presentation has changes that are not saved. Save them with
`save_presentation`, or give `discard: true` to lose them.

`get_basic_info` gives `file` (the path, or null) and `unsaved` (true when
the presentation changed after it was opened or saved).

## Presentations

Click **Present** in the editor to show the slides on the full screen, from
the slide that the editor shows:

- The `auto` videos and shaders of a slide start when the slide shows.
- A click, →, Space or Page Down starts the next `on_click` fill of the
  slide. The fills start in layer order, from the bottom. When all `on_click`
  fills started, the same keys go to the next slide.
- A click on a video or a shader that started pauses it or plays it again.
- ← and Page Up go to the previous slide. Esc stops the presentation.
- D shows or hides the performance figures: the frames per second of the
  screen, the new pictures and the decoded video frames per second, the
  skipped shader frames per second, and the time of each step of a frame.

## Rotation

- `frame.rotation` is an angle in degrees. A positive angle turns the element
  clockwise around the center of its frame.
- The rotation is the final angle, not a change. If you send the same
  `set_frame` two times, the result is the same as one time.
- The editor keeps the angle between -180 (not included) and 180. For
  example, 270 becomes -90.
- `x`, `y`, `width` and `height` are the frame before it turns.
- When a rotated text changes size to fit its content, its rotated top-left
  corner stays in position.
- For a rotated element, `get_elements` and `find_elements` also give
  `bounds`: the unrotated box around the element, in slide units. Use it to
  find overlaps and elements that go past the edge of the slide.
- If a child turns by an angle that is not a multiple of 90° in its group,
  a resize of the group that is not proportional changes the child
  approximately. A box cannot stretch along an axis that is not its own.

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
| `render_failed` | The editor cannot render the slide or the shader video. |
| `operation_failed` | An operation cannot apply. `data.op` gives its index. |
| `operation_failed` with "no cell at row R, column C" | The cell is not in the table, or a merge covers it. |
| `operation_failed` with "the edit would cut a merged cell" | Move the complete merge, or split it first. |
| `stale_revision` | The document changed after `base_revision`. |
| `no_presentation` | The window shows the Home screen. Call `new_presentation` or `open_presentation`. |
| `unsaved_changes` | The open presentation has unsaved changes and the call gives no `discard: true`. |
| `no_file` | `save_presentation` gets no `path` and the presentation has no file. |
| `cannot_open` | `open_presentation` cannot read the file. |
| `io` | The editor cannot write the file. |
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
