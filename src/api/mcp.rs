//! `sliderino mcp`: a Model Context Protocol server on stdio for agents.
//!
//! It lists the tools of the registry and forwards each call to the editor
//! instance through a [`Session`], kept open so the editor shows the agent
//! as connected. The agent's client starts and stops this process; the
//! editor does not need to be open until a tool needs it.

use std::sync::{Arc, Mutex};

use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt as _};
use serde_json::{Value, json};

use crate::api::client::Session;
use crate::api::protocol::{ApiError, ClientKind, ToolOutput};
use crate::api::{skill, tools};

const INSTRUCTIONS: &str = "Sliderino is a presentation editor. These tools read and change the presentation open in the Sliderino editor, live: the person sees each change and shares one undo history with you. Start with get_basic_info, look with get_screenshot, change with apply_operations (one call is one undo step), and check get_diagnostics for text that overflows its box. If no editor is open, call open_editor. For the full guide, read the resource skill://sliderino/SKILL.md.";

/// Runs the server until the client closes stdin.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let service = Server::default().serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}

#[derive(Clone, Default)]
struct Server {
    /// Made on the first call, when the client's name is known.
    session: Arc<Mutex<Option<Session>>>,
}

fn tool_list() -> Vec<Tool> {
    tools::specs()
        .into_iter()
        .map(|spec| {
            let schema = match spec.input_schema() {
                Value::Object(schema) => schema,
                _ => unreachable!("tool schemas are objects"),
            };
            Tool::new(spec.name, spec.description, schema)
                .with_annotations(ToolAnnotations::new().read_only(spec.read_only))
        })
        .collect()
}

/// How long clients may keep the tool list: it only changes with the
/// binary.
const TOOLS_TTL_MS: u64 = 60 * 60 * 1000;

/// The `tools/list` result. MCP 2026-07-28 requires `ttlMs` and
/// `cacheScope`, which rmcp leaves out by default.
fn tool_list_result() -> ListToolsResult {
    ListToolsResult::with_all_items(tool_list())
        .with_ttl_ms(TOOLS_TTL_MS)
        .with_cache_scope(CacheScope::Public)
}

/// The `resources/list` result: the skill. Like the tool list, it only
/// changes with the binary.
fn resource_list_result() -> ListResourcesResult {
    let resource = Resource::new(skill::URI, "SKILL.md")
        .with_title("Sliderino skill")
        .with_description("How to use the Sliderino tools: the workflow, the operations and their fields, and the errors.")
        .with_mime_type("text/markdown")
        .with_size(skill::SKILL.len() as u64);
    ListResourcesResult::with_all_items(vec![resource])
        .with_ttl_ms(TOOLS_TTL_MS)
        .with_cache_scope(CacheScope::Public)
}

/// The `resources/read` result for `uri`.
fn read_resource_result(uri: &str) -> Result<ReadResourceResult, ErrorData> {
    if uri != skill::URI {
        return Err(ErrorData::resource_not_found(
            format!("no resource {uri}"),
            Some(json!({"uri": uri})),
        ));
    }
    let contents = ResourceContents::text(skill::SKILL, skill::URI).with_mime_type("text/markdown");
    Ok(ReadResourceResult::new(vec![contents])
        .with_ttl_ms(TOOLS_TTL_MS)
        .with_cache_scope(CacheScope::Public))
}

/// The MCP form of a tool result: the JSON as text, then the image.
fn to_result(outcome: Result<ToolOutput, ApiError>) -> CallToolResult {
    match outcome {
        Ok(output) => {
            let text = serde_json::to_string_pretty(&output.value).expect("JSON serializes");
            let mut content = vec![ContentBlock::text(text)];
            if let Some(image) = output.image {
                content.push(ContentBlock::image(image.data, image.mime));
            }
            CallToolResult::success(content)
        }
        Err(error) => {
            let text =
                serde_json::to_string_pretty(&json!({"error": error})).expect("JSON serializes");
            CallToolResult::error(vec![ContentBlock::text(text)])
        }
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("sliderino", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(tool_list_result())
    }

    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(resource_list_result())
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        read_resource_result(&request.uri).map(Into::into)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let client = context
            .client_info()
            .map_or_else(|| "Agent".to_string(), |info| info.name);
        let tool = request.name.to_string();
        let args = request.arguments.map_or(Value::Null, Value::Object);
        let session = self.session.clone();
        // Calls block on the socket: keep them off the protocol task.
        let outcome = tokio::task::spawn_blocking(move || {
            let mut session = session.lock().expect("session lock");
            session
                .get_or_insert_with(|| Session::new(client, ClientKind::Mcp))
                .call(&tool, args)
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(to_result(outcome).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_listed_with_an_object_schema() {
        let listed = tool_list();
        assert_eq!(listed.len(), tools::specs().len());
        let screenshot = listed
            .iter()
            .find(|tool| tool.name == "get_screenshot")
            .unwrap();
        assert!(screenshot.input_schema["properties"]["instance"].is_object());
        let open = listed
            .iter()
            .find(|tool| tool.name == "open_editor")
            .unwrap();
        assert!(open.input_schema["properties"].get("instance").is_none());
    }

    #[test]
    fn the_tool_list_says_how_long_to_cache_it() {
        let listed = serde_json::to_value(tool_list_result()).unwrap();
        assert_eq!(listed["ttlMs"], TOOLS_TTL_MS);
        assert_eq!(listed["cacheScope"], "public");
    }

    #[test]
    fn the_skill_is_the_only_resource() {
        let listed = serde_json::to_value(resource_list_result()).unwrap();
        assert_eq!(listed["resources"].as_array().unwrap().len(), 1);
        assert_eq!(listed["resources"][0]["uri"], skill::URI);
        assert_eq!(listed["resources"][0]["name"], "SKILL.md");
        assert_eq!(listed["resources"][0]["mimeType"], "text/markdown");
        assert_eq!(listed["ttlMs"], TOOLS_TTL_MS);
        assert_eq!(listed["cacheScope"], "public");
    }

    #[test]
    fn reading_the_skill_gives_its_text() {
        let read = serde_json::to_value(read_resource_result(skill::URI).unwrap()).unwrap();
        assert_eq!(read["contents"][0]["text"], skill::SKILL);
        assert_eq!(read["contents"][0]["mimeType"], "text/markdown");
        assert_eq!(read["ttlMs"], TOOLS_TTL_MS);
    }

    #[test]
    fn reading_an_unknown_resource_fails() {
        let error = read_resource_result("skill://sliderino/OTHER.md").unwrap_err();
        assert_eq!(error.code, rmcp::model::ErrorCode::RESOURCE_NOT_FOUND);
    }

    #[test]
    fn errors_are_tool_errors_with_their_code() {
        let result = to_result(Err(ApiError::new("no_instance", "no editor")));
        assert_eq!(result.is_error, Some(true));
        let ContentBlock::Text(text) = &result.content[0] else {
            panic!("text content");
        };
        assert!(text.text.contains("no_instance"));
    }
}
