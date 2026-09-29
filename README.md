# Sliderino — Presentation software for the age of agents.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.png">
    <img src="docs/assets/banner-light.png" alt="Sliderino — Slides you make. Slides your agent makes.">
  </picture>
</p>

<p align="center">
  <a href="https://github.com/tncardoso/sliderino/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/tncardoso/sliderino/ci.yml?branch=main&style=flat-square&label=ci" alt="CI status"></a>
  <a href="https://github.com/tncardoso/sliderino/releases/latest"><img src="https://img.shields.io/github/v/release/tncardoso/sliderino?style=flat-square&label=release" alt="Latest release"></a>
  <a href="https://crates.io/crates/sliderino"><img src="https://img.shields.io/crates/v/sliderino?style=flat-square&label=crates.io" alt="crates.io version"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License: MIT"></a>
</p>

<p align="center">
  <a href="https://sliderino.embornal.com">Website</a> ·
  <a href="https://sliderino.embornal.com/docs/">Documentation</a> ·
  <a href="docs/agent-api.md">Agent API</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

Sliderino is presentation software for the age of agents. It has a simple
visual editor. External agents get complete control through a command-line
interface (CLI) and a Model Context Protocol (MCP) server. A presentation is
one self-contained `.sldr` file that you own.

## Install

The releases hold binaries for Linux (x86_64) and macOS (Apple silicon). Run
the installer:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/tncardoso/sliderino/releases/latest/download/sliderino-installer.sh | sh
```

The installer puts `sliderino` in `~/.local/bin`.

Sliderino uses GStreamer to read and play videos. Install it first:

```sh
sudo apt install gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav   # Debian, Ubuntu
brew install gstreamer                                                                                            # macOS
```

To install from crates.io, see [Development](#development).

## Quick start

```sh
sliderino                                  # open the Home screen
sliderino deck.sldr                        # open a file
claude mcp add sliderino -- sliderino mcp  # connect Claude Code
```

Then give the agent a task. For example:

> Open Sliderino and make a five-slide Q3 review. Put the three KPIs on
> slide 2, then check for overflow.

The CLI gives the same tools as the MCP server. Each command writes JSON to
the standard output:

```sh
sliderino info                                # slides, fonts, undo state
sliderino apply ops.json --label "Add KPIs"   # one undo step
sliderino screenshot -o slide.png             # render a slide to PNG
sliderino export-pptx deck.sldr -o deck.pptx  # editable PowerPoint
sliderino tools                               # list every tool
```

## How it fits together

- **Editor**: the visual editor. You and your agents edit the same
  presentation, live, and share one undo history.
- **Agent API**: the editor accepts calls from agents. Each call to
  `apply_operations` is one undo step. Diagnostics tell an agent when text
  overflows its box.
- **MCP server**: `sliderino mcp` gives the agent API to an MCP client, such
  as Claude Code.
- **CLI**: the same tools as the MCP server, for scripts and for agents that
  use the shell.
- **`.sldr` file**: one file that holds the slides with their fonts, images
  and videos.
- **Export**: an editable PowerPoint file (`.pptx`).

## Documentation

| If you want to…                         | Read                                             |
| --------------------------------------- | ------------------------------------------------ |
| Install Sliderino and make a first deck | [Getting started](docs/getting-started.md)       |
| Know what Sliderino is meant to be      | [Vision](docs/VISION.md)                         |
| Know the tools that CLI and MCP give    | [Agent API](docs/agent-api.md)                   |


The documentation is also on the website, at
<https://sliderino.embornal.com/docs/>.

## Development

To build from source, install a Rust toolchain of version 1.98 or newer and
the development files of GStreamer. On Debian and Ubuntu, these are
`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev`. GPUI also needs
`libxkbcommon-dev`, `libxkbcommon-x11-dev`, `libxcb1-dev`, `libwayland-dev` and
`libfontconfig-dev`.

Install the latest release from crates.io:

```sh
cargo install sliderino
```

Or build from a clone of the repository:

```sh
git clone https://github.com/tncardoso/sliderino.git
cd sliderino
cargo run --release
```

Before you send a change, run `cargo clippy`, `cargo fmt` and `cargo test`.

## License

MIT. See [LICENSE](LICENSE).
