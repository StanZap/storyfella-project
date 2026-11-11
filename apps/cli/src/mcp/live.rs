use std::{path::Path, sync::Arc};

use agentkit_mcp::{
    descriptor_to_rmcp_tool,
    rmcp::{
        self,
        model::{
            CallToolRequestParams, CallToolResult, Content, ErrorData, ListToolsResult,
            PaginatedRequestParams, ServerCapabilities, ServerInfo,
        },
        service::{RequestContext, RoleServer},
        ServerHandler, ServiceExt,
    },
};
use anyhow::{ensure, Context, Result};
use serde_json::json;
use storyfella_studio_core::desktop_bridge::{Client, Command, Failure, EDIT_BYTES};

use super::{tool, McpPermission, McpToolDescriptor, ProjectFolder};

pub(super) async fn run(folder: &Path, channel: &Path) -> Result<()> {
    ensure!(folder.is_dir(), "Choose the shared project open in Studio");
    let folder = ProjectFolder::locate_for_authoring(folder)?
        .context("Choose the shared project open in Studio")?;
    let client = Client::connect(&folder, channel)
        .context("Start Studio with a private MCP channel and open the selected project")?;
    Bridge(Arc::new(client))
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[derive(Clone)]
struct Bridge(Arc<Client>);

fn descriptors() -> Vec<McpToolDescriptor> {
    let session = json!({"type":"string","format":"uuid"});
    let offset = json!({"type":"integer","minimum":0});
    vec![
            tool("storyfella_draft_inspect", "Inspect the open Writer entry and get its session, current draft version, saved baseline digest and edit scope.", McpPermission::ReadOnly, json!({}), &[]),
            tool("storyfella_draft_read", "Read the Writer entry or a reachable project-relative file, including unsaved edits. Offsets and lengths are UTF-8 bytes.", McpPermission::ReadOnly, json!({
                "session": session, "offset": offset, "file": {"type":"string"}
            }), &["session", "file"]),
            tool("storyfella_draft_edit", "Validate a replacement in the Writer entry or one reachable included file against the current workspace version. Set apply to true to apply a valid change as one Studio undo step. Studio's normal save settings apply.", McpPermission::Mutating, json!({
                "session": session, "expected_version": {"type":"string","minLength":64,"maxLength":64},
                "start": offset, "end": offset, "text": {"type":"string","maxLength":EDIT_BYTES}, "apply": {"type":"boolean"}, "file": {"type":"string"}
            }), &["session", "file", "expected_version", "start", "end", "text", "apply"]),
            tool("storyfella_preview_start", "Start an isolated Preview of the current Writer draft with pinned artwork. Replaces this connection's previous Preview.", McpPermission::ReadOnly, json!({
                "session":session
            }), &["session"]),
            tool("storyfella_preview_step", "Inspect, advance, choose, checkpoint or restore an isolated Preview. Choice indices are zero-based.", McpPermission::ReadOnly, json!({
                "session":session, "preview":session, "action":{"oneOf":[
                    {"type":"object","properties":{"kind":{"enum":["inspect","advance","checkpoint","restore"]}},"required":["kind"],"additionalProperties":false},
                    {"type":"object","properties":{"kind":{"const":"choose"},"index":offset},"required":["kind","index"],"additionalProperties":false}
                ]}
            }), &["session","preview","action"]),
            tool("storyfella_preview_capture", "Render the current isolated Preview stage as a native PNG on macOS. Width and height describe the Preview viewport in points.", McpPermission::ReadOnly, json!({
                "session":session,"preview":session,
                "width":{"type":"integer","minimum":360,"maximum":960},
                "height":{"type":"integer","minimum":360,"maximum":960}
            }), &["session","preview","width","height"]),
        ]
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = descriptors()
            .iter()
            .map(descriptor_to_rmcp_tool)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                ErrorData::internal_error("Could not build the live Studio tool schemas", None)
            })?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let arguments = request.arguments.unwrap_or_default();
        let (domain, operation) = match request.name.as_ref() {
            "storyfella_draft_inspect" => ("writer", "inspect"),
            "storyfella_draft_read" => ("writer", "read"),
            "storyfella_draft_edit" => ("writer", "edit"),
            "storyfella_preview_start" => ("preview", "start"),
            "storyfella_preview_step" => ("preview", "step"),
            "storyfella_preview_capture" => ("preview", "capture"),
            _ => return Err(ErrorData::invalid_params("Unknown live Studio tool", None)),
        };
        let command = json!({(domain): {(operation): arguments}});
        let client = self.0.clone();
        let reply = tokio::task::spawn_blocking(move || {
            serde_json::from_value::<Command>(command)
                .map_err(|_| {
                    Failure::new(
                        "invalid_arguments",
                        "Use the tool's declared argument schema.",
                    )
                })
                .and_then(|command| client.call(command))
        })
        .await
        .map_err(|_| ErrorData::internal_error("Studio MCP bridge stopped", None))?;
        Ok(match reply {
            Ok(mut value) => {
                let png = value.as_object_mut().and_then(|value| value.remove("png"));
                let mut result = CallToolResult::structured(value);
                if let Some(serde_json::Value::String(png)) = png {
                    result.content.push(Content::image(png, "image/png"));
                }
                result
            }
            Err(error) => CallToolResult::structured_error(json!(error)),
        })
    }
}
