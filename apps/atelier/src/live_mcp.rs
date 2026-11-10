//! Read the project-owned drawing state, including open and parked tabs.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use agentkit_mcp::rmcp::{
    self,
    model::{
        CallToolRequestParams, CallToolResult, ErrorData, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
    ServerHandler, ServiceExt,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use storyfella_studio_core::{
    desktop_bridge::{self, Call, Failure, ProjectIdentity, Reply, MAX_CONNECTIONS, SCHEMA},
    project::ProjectFolder,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{app::DrawingApp, document::Document};

const INSPECT: &str = "atelier_drawing_inspect";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Inspect {}

pub(crate) struct Bridge {
    folder: ProjectFolder,
    calls: mpsc::Receiver<Call<Inspect>>,
}

impl Bridge {
    pub(crate) fn start() -> Result<Option<Self>> {
        let Some(folder) = std::env::var_os("ATELIER_MCP_FOLDER") else {
            return Ok(None);
        };
        let folder = PathBuf::from(folder);
        ensure!(
            folder.is_absolute() && folder.is_dir(),
            "Choose an absolute shared project folder"
        );
        let folder = crate::ownership::target(&folder)?
            .1
            .context("Choose a shared project folder")?;
        let directory = PathBuf::from(
            std::env::var_os("ATELIER_MCP_CHANNEL")
                .context("Set ATELIER_MCP_CHANNEL to a new private channel directory")?,
        );
        let (sender, calls) = mpsc::channel(MAX_CONNECTIONS);
        std::thread::Builder::new()
            .name("atelier-mcp".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::from)
                    .and_then(|runtime| {
                        runtime.block_on(desktop_bridge::serve(&directory, sender))
                    });
                if let Err(error) = result {
                    eprintln!("Atelier MCP channel unavailable: {error:#}");
                }
            })?;
        Ok(Some(Self { folder, calls }))
    }

    pub(crate) fn dispatch(&mut self, app: &DrawingApp) {
        for _ in 0..MAX_CONNECTIONS {
            let Ok(call) = self.calls.try_recv() else {
                break;
            };
            if call.response.is_closed() {
                continue;
            }
            let result = self.inspect(app, &call);
            let (epoch, reply) = match result {
                Ok((epoch, value)) => (Some(epoch), Ok(value)),
                Err(error) => (call.epoch, Err(error)),
            };
            let _ = call.response.send((epoch, reply));
        }
    }

    fn inspect(&self, app: &DrawingApp, call: &Call<Inspect>) -> Result<(Uuid, Value), Failure> {
        if call.request.schema != SCHEMA
            || call.request.project != ProjectIdentity::from(&self.folder)
        {
            return Err(Failure::new("project_mismatch", "Connect to the channel for the selected shared project using a compatible Atelier CLI."));
        }
        let active = app
            .project_owner
            .as_ref()
            .map(|owner| (owner, &app.doc, app.dirty, app.revision));
        let parked = app
            .project_tabs
            .iter()
            .filter_map(|tab| Some((tab.owner.as_ref()?, &tab.doc, tab.dirty, tab.revision)));
        let selected =
            active
                .into_iter()
                .chain(parked)
                .find_map(|(owner, doc, dirty, revision)| {
                    Some((
                        owner.desktop_epoch_for(&self.folder)?,
                        owner,
                        doc,
                        dirty,
                        revision,
                    ))
                });
        let current = selected.as_ref().map(|(epoch, ..)| *epoch);
        if call.epoch.is_some() && call.epoch != current {
            return Err(Failure::new(
                "owner_changed",
                "Reopen the selected drawing and reconnect this MCP client.",
            ));
        }
        let (epoch, owner, doc, dirty, revision) = selected
            .filter(|_| !app.closing)
            .ok_or_else(|| Failure::new("owner_unavailable", "Open the selected drawing in the Atelier window that granted access, then inspect again."))?;
        owner.verify().map_err(|_| {
            Failure::new(
                "project_moved",
                "The shared drawing moved or was replaced. Reopen it in Atelier and reconnect.",
            )
        })?;
        Ok((
            epoch,
            describe(doc, owner.project_id(), dirty, revision, call.session),
        ))
    }
}

fn describe(doc: &Document, project_id: Uuid, dirty: bool, revision: u64, session: Uuid) -> Value {
    let current =
        doc.canvas_created
            .then_some((doc.canvas_id, doc.width, doc.height, doc.layers.as_slice()));
    let parked = doc
        .canvases
        .iter()
        .map(|board| (board.id, board.width, board.height, board.layers.as_slice()));
    let mut artboards: Vec<_> = current.into_iter().chain(parked).collect();
    // Selecting a canvas moves it between the current slot and this list.
    artboards.sort_unstable_by_key(|board| board.0);
    let artboards: Vec<_> = artboards.into_iter().map(|(id, width, height, layers)| {
        let asset = doc.asset_links.iter().find(|link| link.canvas_id == id).map(|link| link.asset_id);
        json!({
            "artboard": id, "asset": asset, "width": width, "height": height,
            "has_candidate": layers.iter().any(|layer| layer.candidate),
            "has_content": layers.iter().any(|layer| layer.image.is_some() || !layer.strokes.is_empty()),
            "layer_count": layers.len(),
        })
    }).collect();
    json!({
        "schema_version": SCHEMA, "session": session, "project_id": project_id,
        "revision": revision, "dirty": dirty, "artboards": artboards, "can_edit": false,
    })
}

pub(crate) async fn run_client(folder: ProjectFolder, channel: &Path) -> Result<()> {
    let client = desktop_bridge::Client::connect(&folder, channel)
        .context("Start Atelier with a private MCP channel and open the selected drawing")?;
    Client(Arc::new(client))
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[derive(Clone)]
struct Client(Arc<desktop_bridge::Client>);

impl Client {
    fn forward(&self, arguments: Value) -> Reply {
        let command: Inspect = serde_json::from_value(arguments).map_err(|_| {
            Failure::new(
                "invalid_arguments",
                "Drawing inspection takes no arguments.",
            )
        })?;
        self.0.call(command)
    }
}

impl ServerHandler for Client {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let schema = json!({ "type": "object", "additionalProperties": false });
        Ok(ListToolsResult::with_all_items(vec![Tool::new(
            INSPECT,
            "Inspect the artboards of the open shared drawing, including unsaved work.",
            schema
                .as_object()
                .expect("tool schema is an object")
                .clone(),
        )]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if request.name != INSPECT {
            return Err(ErrorData::invalid_params("Unknown live Atelier tool", None));
        }
        let client = self.clone();
        let reply = tokio::task::spawn_blocking(move || {
            client.forward(Value::Object(request.arguments.unwrap_or_default()))
        })
        .await
        .map_err(|_| ErrorData::internal_error("Atelier MCP bridge stopped", None))?;
        Ok(match reply {
            Ok(value) => CallToolResult::structured(value),
            Err(failure) => CallToolResult::structured_error(json!(failure)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_inspection_keeps_artboards_in_id_order_when_selection_changes() {
        let mut doc = Document::default();
        for _ in 0..3 {
            doc.create_canvas().unwrap();
        }
        let project = Uuid::new_v4();
        let session = Uuid::new_v4();
        let before = describe(&doc, project, true, 1, session);
        assert_eq!(before["artboards"].as_array().unwrap().len(), 3);
        assert_eq!(before["artboards"][0]["artboard"], 1);
        doc.select_canvas(1).unwrap();
        assert_eq!(describe(&doc, project, true, 1, session), before);
    }
}
