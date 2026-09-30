# Changelog

## [Unreleased]

### Added

- A skill for agents, `SKILL.md`, that tells how to use the Sliderino tools.
  The MCP server gives it as the resource `skill://sliderino/SKILL.md`, and
  `sliderino skill` writes it to stdout. To install it for Claude Code, run
  `sliderino skill > ~/.claude/skills/sliderino/SKILL.md`.

### Fixed

- The tool descriptions that MCP clients and `sliderino tools` show now match
  what the tools do. For example, `get_basic_info` lists the videos,
  `list_fonts` tells when the fonts are still loading, and `save_presentation`
  and `export_pptx` tell that they add the file extension.

## [0.2.0] - 2026-09-28

### Added

- The Sliderino website at <https://sliderino.embornal.com>, with the
  documentation at <https://sliderino.embornal.com/docs/>.

### Removed

- The Components tab of the left panel and the Insert component button of the
  toolbar. They did not do an operation.

## [0.1.0] - 2026-09-28

### Added

- The Table tool. Click to add a table of 3 × 3 cells, or drag to choose
  the number of rows and columns. A table is always as small as its text:
  each column is as wide as its widest cell and each row as tall as its
  tallest cell. Drag a handle to give it more space; the extra space goes to
  all columns or rows in equal parts.
- Double-click a cell to edit its text. Tab and Shift+Tab go to the next and
  the previous cell. Click, Shift+click or drag to select cells; type to
  replace the text of a cell, and press Delete to clear the selected cells.
- Insert, delete and move rows and columns from the context menu, the Table
  section of the properties panel, or the "+" buttons that show next to a
  selected table. Drag selected rows or columns to move them.
- Merge and split cells. Copy and paste cells, also to and from
  spreadsheets. Paste tab-separated text on the canvas to make a new table.
- Set the text style, the fill and the stroke of a whole table, or of the
  selected cells only. Choose the sides of the cells that get the stroke.
- Agents can add and edit tables: see "Tables" in the agent API.

- Select more than one slide in the Slides tab: Ctrl+click adds or removes
  a slide, and Shift+click selects a range of slides.
- Right-click a slide thumbnail to add a new slide, duplicate the selected
  slides or delete them. When you delete all slides, an empty slide stays.
  Undo restores the slides and their selection.
- Drag slide thumbnails to change the order of the slides. When you drag a
  selected slide, all selected slides move together.
- Agents can copy a slide with the `duplicate_slide` operation.
  `get_selection` gives the slides that are selected in the Slides tab in
  `slides`.
- The Home screen. `sliderino` opens on it. Start a new presentation
  (Ctrl+N) or open a file (Ctrl+O) there.
- Save presentations as `.sldr` files, with their fonts, images and videos.
  Ctrl+S saves, and Ctrl+Shift+S saves to a new file. Open a file from the
  Home screen, with Ctrl+O in the editor, or with `sliderino deck.sldr`.
- The editor shows a dot after the name of a presentation with unsaved
  changes. Before Home, a new or an opened presentation, or closing the
  window drops these changes, Sliderino asks to save or discard them.
- Agents can save and open presentations with `save_presentation`,
  `open_presentation` and `new_presentation` (`sliderino save`,
  `sliderino open-file` and `sliderino new`). `get_basic_info` gives the
  file and the unsaved state.

- Video fills: fill a rectangle or an ellipse with a video. Drop a video
  file on the canvas, or pick Video in the fill of the Design tab. Videos in
  other formats than MP4 (H.264 and AAC) change to MP4 when you add them;
  the top bar shows the progress. Set the fit, the opacity, when the video
  starts (Auto or On click), Loop and Sound.
- Shader fills: fill a shape with a GLSL shader written as on Shadertoy
  (`mainImage`, `iTime`, `iResolution`, `iChannel0` and more). Edit the
  source in the Design tab; the problems of the source show with their line.
  Give the shader an image as `iChannel0`, and set its duration, Start and
  Loop.
- Preview in the Design tab plays the selected videos and shaders on the
  canvas, without sound.
- Present shows the slides on the full screen. Videos and shaders start when
  the slide shows or on a click, in layer order. Click a video or a shader
  to pause it. Use the arrow keys to go between slides and Esc to stop.
  Press D to show the frames per second and the time of each step.
- Agents can add videos with `add_video`, fill shapes with `video` and
  `shader`, show shaders at a time with the `time` of `get_screenshot`, and
  make the MP4 of a shader with `render_shader_video`
  (`sliderino shader-video`). `get_diagnostics` reports shaders that do not
  compile and videos that cannot play.
- Videos need GStreamer with its base, good and bad plugins, and an H.264
  and an AAC encoder (the ugly plugins or libav) to change other formats.
  Shaders need a GPU.
- Upload font files (TTF, OTF or TTC). Click the upload button next to the
  font family to give the font to the selected texts, or drop the files on
  the canvas. The font goes into the presentation. If an installed or
  embedded font already has the family name, the uploaded font gets a new
  name, such as "Roboto (2)".
- The family list shows the fonts of the presentation first, under "In this
  presentation".
- The Design tab with nothing selected shows the fonts of the presentation
  and their size. Remove a font there; its texts change to Inter.
- Agents can add a font file with `add_font` and `path` or `data`, and
  remove a font family with `remove_font` and `family`.
- Text tool: click to add a text box that grows with its text, or drag to
  add a box with a fixed size. Text in a narrow box goes onto more lines.
- Design tab for text: font, style, size, line height, letter spacing,
  alignment, paragraph spacing, underline, strikethrough, uppercase, color
  and opacity. Installed fonts are available and go into the presentation.
- Undo and redo for all changes, and a list of the changes in the History
  tab.
- Diagnostics show when text does not fit its box or when the font does not
  have a character.
- `sliderino-debug render` makes a PNG image of a slide from a scene file.
- `sliderino-debug scene` opens a scene file in the editor and can capture
  the slide to a PNG image.
- Press Esc while you move or resize a text box to cancel the change.
- Agents can read and change the open presentation. Add the MCP server with
  `claude mcp add sliderino -- sliderino mcp`, or use the `sliderino` CLI
  (`info`, `screenshot`, `apply`, `undo`, `call`). Agent changes appear
  immediately and go into the same undo history as your changes. See
  `docs/agent-api.md`.
- The agent status at the top of the window shows the connected agents.
  Click it and set **Follow agent** to see the slide and the element of each
  agent change.
- Groups. Press Ctrl+G to put the selected elements into a group and
  Ctrl+Shift+G to ungroup. Click a group to select it, double-click to select
  an element in it, or Ctrl+click to select the element directly. Press
  Enter to select the elements of a group and Esc to select the group again.
- Move a group to move all its elements. Resize a group to scale the
  positions and boxes of its elements; the font sizes do not change.
- Select more than one element: Shift+click to add or remove an element, or
  drag on an empty area to select the elements that the rectangle touches.
  Move, resize, align and delete act on all selected elements.
- Hierarchy tab in the left panel: the elements of the slide as a tree, with
  the top element first. Click a row to select, Shift+click to select a
  range and Ctrl+click to add a row. Drag rows to change the order or to
  move elements into and out of groups.
- Hide and lock elements with the eye and the lock in the Hierarchy tab.
  Hidden elements do not show in the slide or in exports. Locked elements
  cannot change until you unlock them.
- Right-click an element in the Hierarchy tab or on the slide to rename,
  group, ungroup, hide, lock or delete it.
- Agents can add groups and use the `group`, `ungroup`, `move_element` and
  `set_layer` operations.
- Rotate text boxes and groups. Drag just outside a corner of the
  selection to turn it; hold Shift to turn in steps of 15°. Near 0°, 90°,
  180° and 270° the angle snaps to the exact value. You can also type the
  angle in the rotation field of the Design tab. With more than one element
  selected, the field turns each element around its own center.
- A rotated group keeps its angle: its box and its handles turn with it, and
  a resize scales it along its own axes.
- Agents can rotate text and groups with the `rotation` of `set_frame`. The
  angle is the final angle, so the same operation gives the same result each
  time. `get_elements` and `find_elements` give the `bounds` of rotated
  elements.
- Rectangles, ellipses and lines. Pick a tool and drag on the slide, or
  click to add a shape of the default size. Hold Shift to draw a square, a
  circle or a line at a multiple of 45°, and Alt to draw from the center.
  Drag an end of a selected line to move it.
- Shapes have a fill: a color, a linear or radial gradient with up to 10
  colors, or an image. They have a stroke with a color, a width and a solid,
  dashed or dotted style. Rectangles can have round corners, and lines can
  have arrowheads in three sizes at each end.
- Select several shapes to change their style together. The Design tab
  shows "Mixed" for a value that differs.
- Opacity for all elements, in the position section of the Design tab. The
  opacity of a group applies to all its elements.
- Images. Click the Image button, drop PNG or JPEG files on the slide, or
  paste an image. The image goes into the presentation, so the presentation
  opens on another computer with it. Choose Image as the fill of a shape to
  show an image in it, and choose how it fits: cover, contain or stretch.
- Agents can add shapes and images with `add_element`, `add_image`,
  `set_shape_style` and `set_line_points`. See "Shapes" and "Images" in
  `docs/agent-api.md`.
- Export to PowerPoint: choose **Export › PowerPoint (.pptx)** in the top
  bar. The deck keeps the place, size and rotation of every element, and
  stays editable: texts are text boxes with the same line breaks, tables
  are tables and groups are groups. The fonts go inside the file. Videos and
  shaders play in the slide show, and start as they do in Sliderino. When
  PowerPoint shows something differently, such as text that overflows or a
  turned table, a message tells what.
- Agents export with the `export_pptx` tool, and the CLI with
  `sliderino export-pptx -o deck.pptx`. Give a `.sldr` file to export it
  without opening the editor.

### Changed

- The editor title bar shows Home instead of the folder. Click it to go
  back to the Home screen.
- `open_editor` opens an editor with a new presentation, as before. The
  other tools fail with `no_presentation` when the window shows the Home
  screen.
- The agent tool `get_selection` gives the selected elements as a list.
- Text opacity is now the opacity of the element. Agents set it with the
  `opacity` of `set_layer`, not with the text style.
- Shader fills play smoother in a presentation and in the editor preview:
  the GPU no longer makes the screen wait for a new frame.

### Fixed

- Error messages, such as a file that cannot be read, show in the window.
- Text in an embedded font now shows in the editor when an installed font
  has the same family name, or when the font has no letter "m".
- Moving and resizing a text box is smooth. Before, the editor stopped
  responding during the drag.
- The first selection of a text box no longer waits for the system fonts to
  load.
- The selection rectangle and the outline of each selected element now show
  on the slide.

[Unreleased]: https://github.com/tncardoso/sliderino/compare/v0.1.0...main
[0.1.0]: https://github.com/tncardoso/sliderino/releases/tag/v0.1.0
