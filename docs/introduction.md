# Introduction

Sliderino is presentation software for the age of agents. It has a simple
visual editor. External agents get complete control through a command-line
interface (CLI) and a Model Context Protocol (MCP) server.

A presentation is one self-contained `.sldr` file that you own. The file
holds the slides and their fonts, images and videos. Sliderino needs no
server.

## What you can do

- Make slides in the editor with text, shapes, images, groups, tables,
  videos and shaders.
- Let an agent change the open presentation while you watch. The changes of
  the agent and your changes go into one undo history.
- See text that overflows its box. The editor shows it, and agents get it
  from the `get_diagnostics` tool.
- Present on the full screen.
- Export the presentation to an editable PowerPoint file.

## Where to go next

- [Getting started](getting-started.md): install Sliderino, open the
  editor and connect an agent.
- [Agent API](agent-api.md): the tools that the CLI and the MCP server give.
- [PowerPoint export](pptx-export.md): what the export keeps.
- [Vision](VISION.md): what Sliderino is meant to be.
- [Changelog](changelog.md): the changes in each release.

Contributors can also read [Debug scenes](debug-scenes.md) and
[Releasing](releasing.md).

Sliderino has the MIT license. The source code is on
[GitHub](https://github.com/tncardoso/sliderino).
