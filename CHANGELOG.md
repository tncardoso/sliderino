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

### Fixed

- Moving and resizing a text box is smooth. Before, the editor stopped
  responding during the drag.
- The first selection of a text box no longer waits for the system fonts to
  load.
