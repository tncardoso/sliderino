# Changelog

## [Unreleased]

### Added

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

### Changed

- The agent tool `get_selection` gives the selected elements as a list.

### Fixed

- Moving and resizing a text box is smooth. Before, the editor stopped
  responding during the drag.
- The first selection of a text box no longer waits for the system fonts to
  load.
