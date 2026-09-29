# Getting started

Sliderino runs on Linux (x86_64) and macOS (Apple silicon). Shaders need a
GPU. Videos need GStreamer.

## 1. Install Sliderino

Run the installer:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tncardoso/sliderino/releases/latest/download/sliderino-installer.sh | sh
```

The installer puts `sliderino` in `~/.local/bin`.

Sliderino uses GStreamer to read and play videos. Install it:

```sh
# Debian, Ubuntu
sudo apt install gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-libav

# macOS
brew install gstreamer
```

### Build from source

To build from source, install a Rust toolchain of version 1.98 or newer and
the development files of GStreamer. On Debian and Ubuntu, these are
`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev`. GPUI also
needs `libxkbcommon-dev`, `libxkbcommon-x11-dev`, `libxcb1-dev`,
`libwayland-dev` and `libfontconfig-dev`. Then run:

```sh
cargo install sliderino
```

## 2. Open the editor

```sh
sliderino                 # the Home screen
sliderino deck.sldr       # open a file
```

The Home screen opens. Press Ctrl+N for a new presentation, or Ctrl+O to
open a `.sldr` file.

## 3. Connect your agent

Add the MCP server to your MCP client. For Claude Code:

```sh
claude mcp add sliderino -- sliderino mcp
```

The editor shows each connected client at the top of the window. Then give
the agent a task. For example:

> Open Sliderino and make a five-slide Q3 review. Put the three KPIs on
> slide 2, then check for overflow.

## 4. Use the CLI

The CLI gives the same tools as the MCP server. Each command writes JSON to
the standard output.

```sh
sliderino info                                # slides, fonts, undo state
sliderino apply ops.json --label "Add KPIs"   # one undo step
sliderino screenshot -o slide.png             # render a slide to PNG
sliderino export-pptx deck.sldr -o deck.pptx  # editable PowerPoint
sliderino tools                               # list every tool
```

Run `sliderino --help` to see all commands. The [Agent API](agent-api.md)
tells what each tool does.
