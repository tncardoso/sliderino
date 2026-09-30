//! The agent API: external agents read and change the presentation open in
//! the editor through `sliderino mcp` (Model Context Protocol on stdio) or
//! the `sliderino` CLI. Both are clients of the editor's socket; see
//! `docs/agent-api.md` and `SKILL.md`.

pub mod client;
pub mod mcp;
pub mod ops;
pub mod protocol;
pub mod server;
pub mod skill;
pub mod tools;

/// Version of the tools and of the ops format. Changes that break clients
/// increment it.
pub const API_VERSION: u32 = 1;
