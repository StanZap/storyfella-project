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
use base64::{engine::general_purpose::STANDARD, Engine};
use gpui_kit::{Context as AppContext, Window};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use storyfella_studio_core::{
    desktop_bridge::{self, Call, Failure, Reply, MAX_CONNECTIONS, SCHEMA},
    project::ProjectFolder,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    app::{DrawingApp, Page},
    document::{self, Document},
    workflow::{rect_in_canvas, DrawingTarget, PixelRect, SelectionMode},
};

const INSPECT: &str = "atelier_drawing_inspect";
const SET_MASK: &str = "atelier_artboard_mask_set";
const PREVIEW_REGION: &str = "atelier_artboard_region_preview";

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Inspect {},
    SetMask {
        session: Uuid,
        expected_revision: u64,
        artboard: u32,
        #[serde(deserialize_with = "Deserialize::deserialize")]
        rect: Option<PixelRect>,
    },
    PreviewRegion {
        session: Uuid,
        expected_revision: u64,
        artboard: u32,
        rect: PixelRect,
    },
}

pub(crate) struct Bridge {
    folder: ProjectFolder,
    calls: mpsc::Receiver<Call<Command>>,
    edit: bool,
    render: Arc<()>,
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
        Ok(Some(Self {
            folder,
            calls,
            edit: std::env::var_os("ATELIER_MCP_EDIT").is_some_and(|value| value == "1"),
            render: Arc::new(()),
        }))
    }

    pub(crate) fn dispatch(
        &mut self,
        app: &mut DrawingApp,
        window: &mut Window,
        cx: &mut AppContext<DrawingApp>,
    ) {
        for _ in 0..MAX_CONNECTIONS {
            let Ok(call) = self.calls.try_recv() else {
                break;
            };
            if call.response.is_closed() {
                continue;
            }
            match self.prepare(app, &call) {
                Ok((epoch, Action::Reply(value))) => {
                    let _ = call.response.send((Some(epoch), Ok(value)));
                }
                Ok((epoch, Action::Mask(rect))) => {
                    app.capture_prompt(cx);
                    let target = match rect {
                        Some(rect) => DrawingTarget::Region {
                            mode: SelectionMode::Blend,
                            rect,
                        },
                        None if app.doc.drawing_target.blend_mask().is_some() => {
                            DrawingTarget::Full
                        }
                        None => app.doc.drawing_target,
                    };
                    app.set_drawing_target(target, cx);
                    app.restore_prompt(window, cx);
                    app.capture_prompt(cx);
                    cx.notify();
                    let _ = call.response.send((
                        Some(epoch),
                        Ok(json!({
                            "artboard": app.doc.canvas_id, "revision": app.revision, "mask": rect,
                        })),
                    ));
                }
                Ok((epoch, Action::Render(mut doc, artboard, revision, rect))) => {
                    let render = self.render.clone();
                    std::thread::spawn(move || {
                        let _render = render;
                        if !call.response.is_closed() {
                            let reply = preview_region(&mut doc, artboard, revision, rect);
                            let _ = call.response.send((Some(epoch), reply));
                        }
                    });
                }
                Err(error) => {
                    let _ = call.response.send((call.epoch, Err(error)));
                }
            }
        }
    }

    fn prepare(&self, app: &DrawingApp, call: &Call<Command>) -> Result<(Uuid, Action), Failure> {
        call.request.check_project(&self.folder)?;
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
        let action = match call.request.command {
            Command::Inspect {} => Action::Reply(describe(
                doc,
                owner.project_id(),
                dirty,
                revision,
                call.session,
                self.edit,
            )),
            Command::SetMask {
                session,
                expected_revision,
                artboard,
                rect,
            } => {
                check_revision(call, session, expected_revision, revision)?;
                if !self.edit {
                    return Err(Failure::new(
                        "scope_denied",
                        "Enable Atelier draft edits when opening a new channel.",
                    ));
                }
                if app.page != Page::Draw
                    || !std::ptr::eq(doc, &app.doc)
                    || !doc.canvas_created
                    || doc.canvas_id != artboard
                {
                    return Err(Failure::new(
                        "artboard_inactive",
                        "Open this artboard in Draw before changing its blend mask.",
                    ));
                }
                if app.switching
                    || app.closing_project.is_some()
                    || app.active_stroke
                    || app.transform_original.is_some()
                    || app.artboard_resize_drag.is_some()
                    || app.region_drag.is_some()
                    || app.image_crop.is_some()
                {
                    return Err(Failure::new(
                        "owner_busy",
                        "Finish the current Atelier action and inspect the drawing again.",
                    ));
                }
                if rect.is_some_and(|(_, _, w, h)| w < 16 || h < 16)
                    || rect.is_some_and(|rect| !rect_in_canvas(rect, doc.width, doc.height))
                {
                    return Err(Failure::new(
                        "invalid_rect",
                        "Use a rectangle at least 16 by 16 pixels inside the artboard.",
                    ));
                }
                Action::Mask(rect)
            }
            Command::PreviewRegion {
                session,
                expected_revision,
                artboard,
                rect,
            } => {
                check_revision(call, session, expected_revision, revision)?;
                if !doc.canvas_created && !doc.canvases.iter().any(|canvas| canvas.id == artboard) {
                    return Err(Failure::new(
                        "unknown_artboard",
                        "Inspect the drawing for its current artboards.",
                    ));
                }
                if std::ptr::eq(doc, &app.doc)
                    && (app.active_stroke
                        || app.transform_original.is_some()
                        || app.artboard_resize_drag.is_some())
                {
                    return Err(Failure::new(
                        "owner_busy",
                        "Finish the current drawing gesture and inspect again.",
                    ));
                }
                if Arc::strong_count(&self.render) > 1 {
                    return Err(Failure::new(
                        "owner_busy",
                        "Wait for the current region preview to finish.",
                    ));
                }
                Action::Render(Box::new(doc.clone()), artboard, revision, rect)
            }
        };
        Ok((epoch, action))
    }
}

enum Action {
    Reply(Value),
    Mask(Option<PixelRect>),
    Render(Box<Document>, u32, u64, PixelRect),
}

fn check_revision(
    call: &Call<Command>,
    session: Uuid,
    expected: u64,
    current: u64,
) -> Result<(), Failure> {
    if call.epoch.is_none() || session != call.session {
        return Err(Failure::new(
            "session_changed",
            "Inspect the drawing on this connection first.",
        ));
    }
    if expected != current {
        return Err(Failure {
            current_version: Some(current.to_string()),
            ..Failure::new(
                "stale_revision",
                "The drawing changed. Inspect it for the current revision before retrying.",
            )
        });
    }
    Ok(())
}

fn preview_region(doc: &mut Document, artboard: u32, revision: u64, rect: PixelRect) -> Reply {
    doc.select_canvas(artboard).map_err(|_| {
        Failure::new(
            "unknown_artboard",
            "Inspect the drawing for its current artboards.",
        )
    })?;
    if !rect_in_canvas(rect, doc.width, doc.height) {
        return Err(Failure::new(
            "invalid_rect",
            "Use a nonempty rectangle inside the artboard.",
        ));
    }
    let failed = |_| {
        Failure::new(
            "render_failed",
            "Atelier could not render the requested region.",
        )
    };
    let raster = document::raster(&doc.artwork(), document::MAX_EDGE, true).map_err(failed)?;
    let (x, y, w, h) = rect;
    let crop = image::imageops::crop_imm(&raster, x, y, w, h).to_image();
    let image = document::thumbnail(image::DynamicImage::ImageRgba8(crop), 256);
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| {
            Failure::new(
                "render_failed",
                "Atelier could not encode the region preview.",
            )
        })?;
    Ok(
        json!({"artboard": artboard, "revision": revision, "rect": rect, "mask": doc.drawing_target.blend_mask(),
        "width": image.width(), "height": image.height(), "png": STANDARD.encode(png.into_inner())}),
    )
}

fn describe(
    doc: &Document,
    project_id: Uuid,
    dirty: bool,
    revision: u64,
    session: Uuid,
    can_edit: bool,
) -> Value {
    let current = doc.canvas_created.then_some((
        doc.canvas_id,
        doc.width,
        doc.height,
        doc.layers.as_slice(),
        doc.drawing_target.blend_mask(),
    ));
    let parked = doc.canvases.iter().map(|board| {
        (
            board.id,
            board.width,
            board.height,
            board.layers.as_slice(),
            board.drawing_target.blend_mask(),
        )
    });
    let mut artboards: Vec<_> = current.into_iter().chain(parked).collect();
    // Selecting a canvas moves it between the current slot and this list.
    artboards.sort_unstable_by_key(|board| board.0);
    let artboards: Vec<_> = artboards.into_iter().map(|(id, width, height, layers, mask)| {
        let asset = doc.asset_links.iter().find(|link| link.canvas_id == id).map(|link| link.asset_id);
        json!({
            "artboard": id, "asset": asset, "width": width, "height": height, "mask": mask,
            "has_candidate": layers.iter().any(|layer| layer.candidate),
            "has_content": layers.iter().any(|layer| layer.image.is_some() || !layer.strokes.is_empty()),
            "layer_count": layers.len(),
        })
    }).collect();
    json!({
        "schema_version": SCHEMA, "session": session, "project_id": project_id,
        "revision": revision, "dirty": dirty, "artboards": artboards, "can_edit": can_edit,
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
    fn forward(&self, name: &str, arguments: Value) -> Reply {
        let arguments = match name {
            INSPECT => json!({"inspect": arguments}),
            SET_MASK => json!({"set_mask": arguments}),
            PREVIEW_REGION => json!({"preview_region": arguments}),
            _ => return Err(Failure::new("unknown_tool", "Choose a live Atelier tool.")),
        };
        let command: Command = serde_json::from_value(arguments).map_err(|_| {
            Failure::new(
                "invalid_arguments",
                "Use the tool's declared argument schema.",
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
        let tool = |name, description, properties, required| {
            let schema = json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false});
            Tool::new(
                name,
                description,
                schema
                    .as_object()
                    .expect("tool schema is an object")
                    .clone(),
            )
        };
        let rect = json!({"type":"array","minItems":4,"maxItems":4,"items":{"type":"integer","minimum":0}});
        let mut properties = json!({"session":{"type":"string","format":"uuid"},"artboard":{"type":"integer","minimum":0},
            "expected_revision":{"type":"integer","minimum":0},"rect":rect});
        let required = json!(["session", "artboard", "expected_revision", "rect"]);
        let preview = tool(PREVIEW_REGION, "Render accepted art in an unrotated [x,y,width,height] rectangle at the inspected drawing revision. Returns a native PNG up to 256 pixels on its long edge.", properties.clone(), required.clone());
        properties["rect"] = json!({"anyOf":[rect,{"type":"null"}]});
        Ok(ListToolsResult::with_all_items(vec![
            tool(INSPECT, "Inspect the artboards of the open shared drawing, including unsaved work.", json!({}), json!([])),
            tool(SET_MASK, "Set the active Draw artboard's blend mask in unrotated [x,y,width,height] pixels. Pass null to clear it. Requires the drawing edit grant.", properties, required),
            preview,
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let client = self.clone();
        let reply = tokio::task::spawn_blocking(move || {
            client.forward(
                &request.name,
                Value::Object(request.arguments.unwrap_or_default()),
            )
        })
        .await
        .map_err(|_| ErrorData::internal_error("Atelier MCP bridge stopped", None))?;
        Ok(match reply {
            Ok(mut value) => {
                let png = value.as_object_mut().and_then(|value| value.remove("png"));
                let mut result = CallToolResult::structured(value);
                if let Some(Value::String(png)) = png {
                    result
                        .content
                        .push(agentkit_mcp::rmcp::model::Content::image(png, "image/png"));
                }
                result
            }
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
        let before = describe(&doc, project, true, 1, session, false);
        assert_eq!(before["artboards"].as_array().unwrap().len(), 3);
        assert_eq!(before["artboards"][0]["artboard"], 1);
        doc.select_canvas(1).unwrap();
        assert_eq!(describe(&doc, project, true, 1, session, false), before);
    }

    #[test]
    fn blend_mask_survives_serialization_and_undo_keeps_local_paint() {
        let mut doc = Document::default();
        doc.create_canvas().unwrap();
        doc.layers[0].strokes.push(document::Stroke {
            points: Arc::new(vec![document::Point { x: 12.0, y: 16.0 }]),
            size: 10.0,
            opacity: 1.0,
            brush: document::Brush::Ink,
            pressures: Arc::default(),
            tool: document::Tool::Pencil,
        });
        let mut history = document::History::new();
        history.checkpoint(&doc);
        doc.drawing_target = DrawingTarget::Region {
            mode: SelectionMode::Blend,
            rect: (0, 0, 32, 32),
        };
        let mut recovered: Document =
            serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap();
        assert_eq!(recovered.drawing_target, doc.drawing_target);
        let artboard = doc.canvas_id;
        let reply = preview_region(&mut recovered, artboard, 2, (0, 0, 32, 32)).unwrap();
        let bytes = STANDARD.decode(reply["png"].as_str().unwrap()).unwrap();
        let pixels = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_ne!(pixels.get_pixel(12, 16).0[3], 0);
        assert_eq!(pixels.get_pixel(0, 0).0[3], 0);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.drawing_target, DrawingTarget::Full);
        assert_eq!(doc.layers[0].strokes.len(), 1);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.drawing_target.blend_mask(), Some((0, 0, 32, 32)));
    }

    #[test]
    fn clearing_a_blend_mask_requires_an_explicit_null() {
        let mut request = json!({"set_mask": {
            "session": Uuid::new_v4(), "expected_revision": 2, "artboard": 1
        }});
        assert!(serde_json::from_value::<Command>(request.clone()).is_err());
        request["set_mask"]["rect"] = serde_json::Value::Null;
        assert!(matches!(
            serde_json::from_value(request).unwrap(),
            Command::SetMask { rect: None, .. }
        ));
    }
}
