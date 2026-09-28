# Sliderino

Sliderino is presentation software for the age of agents. It has a simple
visual editor. External agents get complete control through a command-line
interface (CLI) and a Model Context Protocol (MCP) server. A presentation is
one self-contained `.sldr` file that you own. Sliderino can also export a
presentation to PowerPoint.

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

To build from source, install a Rust toolchain of version 1.98 or newer and
the development files of GStreamer. On Debian and Ubuntu, these are
`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev`. GPUI also needs
`libxkbcommon-dev`, `libxkbcommon-x11-dev`, `libxcb1-dev`, `libwayland-dev` and
`libfontconfig-dev`. Then run:

```sh
cargo install sliderino
```

## Use

```sh
sliderino open                 # open the editor
sliderino mcp                  # serve the agent API to an MCP client
sliderino tools                # list the tools of the agent API
```

To connect Claude Code:

```sh
claude mcp add sliderino -- sliderino mcp
```

## Documentation

- [Agent API](docs/agent-api.md): the tools that CLI and MCP give.
- [PowerPoint export](docs/pptx-export.md): what the export keeps.
- [Debug scenes](docs/debug-scenes.md): the `sliderino-debug` tool for
  contributors.
- [Vision](docs/VISION.md): what Sliderino is meant to be.
- [Releasing](docs/releasing.md): how a maintainer makes a release.

## License

MIT. See [LICENSE](LICENSE).
