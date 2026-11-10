use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde_json::json;
use storyfella_studio_core::desktop_bridge::{Client, Command, Failure, EDIT_BYTES};

use super::{
    tool, tool_result, McpError, McpPermission, McpToolCall, McpToolDescriptor, McpToolRegistry,
    McpToolResult, ProjectFolder,
};

pub(super) async fn run(folder: &Path, channel: &Path) -> Result<()> {
    ensure!(folder.is_dir(), "Choose the shared project open in Studio");
    let folder = ProjectFolder::locate_for_authoring(folder)?
        .context("Choose the shared project open in Studio")?;
    let client = Client::connect(&folder, channel)
        .context("Start Studio with a private MCP channel and open the selected project")?;
    agentkit_mcp::run_stdio(Bridge(client)).await?;
    Ok(())
}

struct Bridge(Client);

impl McpToolRegistry for Bridge {
    fn list_tools(&self) -> Vec<McpToolDescriptor> {
        let session = json!({"type":"string","format":"uuid"});
        let offset = json!({"type":"integer","minimum":0});
        vec![
            tool("storyfella_draft_inspect", "Inspect the open Writer entry and get its session, current draft version, saved baseline digest and edit scope.", McpPermission::ReadOnly, json!({}), &[]),
            tool("storyfella_draft_read", "Read the open Writer entry, including unsaved edits. Offsets and lengths are UTF-8 bytes.", McpPermission::ReadOnly, json!({
                "session": session, "offset": offset
            }), &["session"]),
            tool("storyfella_draft_edit", "Validate a replacement in the open Writer entry against its current version and includes. Set apply to true to apply a valid change as one Studio undo step. Studio's normal save settings apply.", McpPermission::Mutating, json!({
                "session": session, "expected_version": {"type":"string","minLength":64,"maxLength":64},
                "start": offset, "end": offset, "text": {"type":"string","maxLength":EDIT_BYTES}, "apply": {"type":"boolean"}
            }), &["session", "expected_version", "start", "end", "text"]),
        ]
    }

    fn call_tool(&self, call: McpToolCall) -> Result<McpToolResult, McpError> {
        let command = match call.name.as_str() {
            "storyfella_draft_inspect" => json!({"inspect": call.arguments}),
            "storyfella_draft_read" => json!({"read": call.arguments}),
            "storyfella_draft_edit" => json!({"edit": call.arguments}),
            _ => return Err(McpError::ToolNotFound { name: call.name }),
        };
        let reply = serde_json::from_value::<Command>(command)
            .map_err(|_| {
                Failure::new(
                    "invalid_arguments",
                    "Use the tool's declared argument schema.",
                )
            })
            .and_then(|command| self.0.call(command));
        Ok(tool_result(call.call_id, call.name, reply))
    }
}
