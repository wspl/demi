//! Inventory and export resources already observed in the current page.

use std::collections::HashMap;

use base64::Engine;
use chromiumoxide::cdp::{
    browser_protocol::{
        network::{LoaderId, ResourceType},
        page::{FrameId, FrameResourceTree, GetResourceContentParams, GetResourceTreeParams},
    },
    js_protocol::runtime::EvaluateParams,
};
use demi_command_service::InvocationContext;
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{
    BrowserEnvironment, BrowserError, BrowserTab, Result,
    operation::Operation,
    protocol::{
        AssetKind, AssetsExportResult, AssetsListResult, BrowserFailure, BrowserOperation,
        ExportedAsset, INLINE_BYTES, InlineSvg, MAX_NODES,
    },
};

/// The manifest an export writes beside the files: what it saved and what failed.
#[derive(Serialize)]
struct Manifest<'a> {
    inventory: &'a str,
    files: &'a [ExportedAsset],
    failures: &'a [ExportFailure],
}

#[derive(Serialize)]
struct ExportFailure {
    id: String,
    error: BrowserFailure,
}

#[derive(Default)]
pub(super) struct State {
    documents: HashMap<FrameId, LoaderId>,
    inventories: HashMap<String, Inventory>,
}

struct Inventory {
    assets: Vec<Asset>,
}

struct Asset {
    id: String,
    kind: AssetKind,
    mime: String,
    source: Source,
}

enum Source {
    Resource {
        page: chromiumoxide::Page,
        frame: FrameId,
        url: String,
    },
    Svg(String),
}

/// Keep resource acquisition on the page whose inventory the caller selected.
pub(super) async fn execute(
    context: &InvocationContext,
    _environment: &BrowserEnvironment,
    tab: Option<&BrowserTab>,
    command: &BrowserOperation,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    let tab = tab.ok_or(BrowserError::TabNotFound)?;
    let operation = Operation::for_tab(tab, cancel, deadline);
    let _guard = tab
        .state
        .operations
        .try_lock()
        .map_err(|_| BrowserError::Busy)?;
    let mut state = tab.state.assets.lock().await;
    let frames = operation.run(inventory_frames(&tab.page)).await?;
    let documents = frames
        .iter()
        .map(|(_, tree)| (tree.frame.id.clone(), tree.frame.loader_id.clone()))
        .collect();
    if state.documents != documents {
        state.inventories.clear();
        state.documents = documents;
    }
    match command {
        BrowserOperation::AssetsList(_) => {
            let handle = super::handles::fresh("assets")?;
            let mut result = AssetsListResult {
                inventory: handle.clone(),
                assets: Vec::new(),
                inline_svgs: Vec::new(),
                truncated: false,
            };
            let mut inventory = Inventory { assets: Vec::new() };
            let mut size = 0;
            for (page, frame) in frames {
                for resource in frame.resources {
                    let kind = match resource.r#type {
                        ResourceType::Font => AssetKind::Font,
                        ResourceType::Image => AssetKind::Image,
                        ResourceType::Stylesheet => AssetKind::Stylesheet,
                        ResourceType::Media if resource.mime_type.starts_with("video/") => {
                            AssetKind::Video
                        }
                        _ => continue,
                    };
                    let id = super::handles::fresh("asset")?;
                    let entry = super::protocol::Asset {
                        id: id.clone(),
                        kind,
                        url: resource.url.clone(),
                        mime_type: Some(resource.mime_type.clone()),
                    };
                    let bytes = serde_json::to_vec(&entry)
                        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
                        .len();
                    if inventory.assets.len() >= MAX_NODES || size + bytes > INLINE_BYTES / 2 {
                        result.truncated = true;
                        continue;
                    }
                    size += bytes;
                    result.assets.push(entry);
                    inventory.assets.push(Asset {
                        id,
                        kind,
                        mime: resource.mime_type,
                        source: Source::Resource {
                            page: page.clone(),
                            frame: frame.frame.id.clone(),
                            url: resource.url,
                        },
                    });
                }
                let context_id = operation
                    .run(async { Ok(page.frame_execution_context(frame.frame.id.clone()).await?) })
                    .await?;
                let Some(context_id) = context_id else {
                    return Err(BrowserError::InvalidResult(
                        "asset frame has no execution context".into(),
                    ));
                };
                let svgs: Vec<String> = operation.run(async {
                    page.evaluate_expression(EvaluateParams::builder()
                        .expression("Array.from(document.querySelectorAll('svg'), svg => svg.outerHTML)")
                        .context_id(context_id).return_by_value(true).build()
                        .map_err(BrowserError::Configuration)?).await?
                        .into_value().map_err(|error| BrowserError::InvalidResult(error.to_string()))
                }).await?;
                for html in svgs {
                    if inventory.assets.len() >= MAX_NODES || size + html.len() > INLINE_BYTES / 2 {
                        result.truncated = true;
                        continue;
                    }
                    size += html.len();
                    let id = super::handles::fresh("asset")?;
                    result.inline_svgs.push(InlineSvg {
                        id: id.clone(),
                        html: html.clone(),
                    });
                    inventory.assets.push(Asset {
                        id,
                        kind: AssetKind::Image,
                        mime: "image/svg+xml".into(),
                        source: Source::Svg(html),
                    });
                }
            }
            state.inventories.insert(handle, inventory);
            super::output::value(result)
        }
        BrowserOperation::AssetsExport(input) => {
            if input.id.is_some() == input.kind.is_some() {
                return Err(BrowserError::Configuration(
                    "asset export requires either --id or --kind".into(),
                ));
            }
            let inventory = state
                .inventories
                .get(&input.inventory)
                .ok_or(BrowserError::StaleInventory)?;
            if let Some(ids) = &input.id {
                for id in ids {
                    if !inventory.assets.iter().any(|asset| &asset.id == id) {
                        return Err(BrowserError::Configuration(format!(
                            "asset {id} is not in this inventory"
                        )));
                    }
                }
            }
            let selected: Vec<_> = inventory
                .assets
                .iter()
                .filter(|asset| {
                    input.id.as_ref().is_some_and(|ids| ids.contains(&asset.id))
                        || input
                            .kind
                            .as_ref()
                            .is_some_and(|kinds| kinds.contains(&asset.kind))
                })
                .collect();
            let directory = super::output::resolve(&context.request.cwd, &input.output_dir)?;
            operation
                .run(async {
                    tokio::fs::create_dir_all(&directory).await?;
                    Ok(())
                })
                .await?;
            let manifest_path = directory.join("manifest.json");
            let overwrite = input.overwrite == Some(true);
            super::output::preflight(
                &context.request.cwd,
                &manifest_path.to_string_lossy(),
                overwrite,
            )
            .await?;
            let mut files = Vec::new();
            let mut failures = Vec::new();
            for asset in selected {
                let acquired = operation
                    .run(async {
                        let bytes = match &asset.source {
                            Source::Svg(html) => html.as_bytes().to_vec(),
                            Source::Resource { page, frame, url } => {
                                resource_bytes(page, frame, url).await?
                            }
                        };
                        Ok(bytes)
                    })
                    .await;
                let saved = match acquired {
                    Ok(bytes) => {
                        let count = bytes.len();
                        let path =
                            directory.join(format!("{}.{}", asset.id, extension(&asset.mime)));
                        super::output::save_with_overwrite(
                            &context.request.cwd,
                            &path.to_string_lossy(),
                            bytes,
                            overwrite,
                            cancel,
                            deadline,
                        )
                        .await
                        .map(|path| ExportedAsset {
                            id: asset.id.clone(),
                            path,
                            bytes: count,
                            mime_type: asset.mime.clone(),
                        })
                    }
                    Err(error) => Err(error),
                };
                match saved {
                    Ok(file) => files.push(file),
                    Err(error) => failures.push(ExportFailure {
                        id: asset.id.clone(),
                        error: BrowserFailure {
                            code: error.code(),
                            message: error.to_string(),
                            details: None,
                        },
                    }),
                }
            }
            let manifest = serde_json::to_vec_pretty(&Manifest {
                inventory: &input.inventory,
                files: &files,
                failures: &failures,
            })
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
            // The manifest is cleanup for a partial export; publish it with its own
            // bounded cleanup token even when collection was cancelled.
            let cleanup_cancel = CancellationToken::new();
            let manifest = super::output::save_with_overwrite(
                &context.request.cwd,
                &manifest_path.to_string_lossy(),
                manifest,
                overwrite,
                &cleanup_cancel,
                tokio::time::Instant::now() + super::operation::CONTROL_TIMEOUT,
            )
            .await?;
            let result = AssetsExportResult {
                directory: directory.to_string_lossy().into_owned(),
                manifest,
                files,
            };
            if cancel.is_cancelled() || tokio::time::Instant::now() >= deadline {
                let cause = if tokio::time::Instant::now() >= deadline {
                    BrowserError::Timeout
                } else {
                    BrowserError::Cancelled
                };
                return Err(BrowserError::Action {
                    source: Box::new(cause),
                    details: super::protocol::ErrorDetails {
                        export: Some(result),
                        ..super::protocol::ErrorDetails::default()
                    },
                });
            }
            if !failures.is_empty() {
                return Err(BrowserError::PartialFailure { export: result });
            }
            super::output::value(result)
        }
        _ => unreachable!("asset dispatch accepts only asset commands"),
    }
}

/// Name exported browser assets from their declared media type, never their URL.
fn extension(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "text/css" => "css",
        "font/woff" => "woff",
        "font/woff2" => "woff2",
        "font/ttf" => "ttf",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        _ => "bin",
    }
}

/// Acquire an observed browser resource through its frame without navigating.
pub(super) async fn resource_bytes(
    page: &chromiumoxide::Page,
    frame: &FrameId,
    url: &str,
) -> Result<Vec<u8>> {
    let content = page
        .execute(GetResourceContentParams::new(frame.clone(), url))
        .await?
        .result;
    if content.base64_encoded {
        base64::engine::general_purpose::STANDARD
            .decode(content.content)
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))
    } else {
        Ok(content.content.into_bytes())
    }
}

/// Collect resource trees from every attached renderer of an asset inventory.
async fn inventory_frames(
    page: &chromiumoxide::Page,
) -> Result<Vec<(chromiumoxide::Page, FrameResourceTree)>> {
    let snapshot = super::frames::capture(page).await?;
    let mut result = Vec::with_capacity(snapshot.frames.len());
    for document in snapshot.frames {
        let owner = document.page;
        // Resource trees omit separate renderer children. The shared snapshot
        // locates them; each resource request runs on that frame's own session.
        let resources = owner
            .execute(GetResourceTreeParams {})
            .await?
            .result
            .frame_tree;
        let mut resources = vec![resources];
        let mut found = None;
        while let Some(mut resource) = resources.pop() {
            resources.extend(resource.child_frames.take().unwrap_or_default());
            if resource.frame.id == document.frame.id {
                found = Some(resource);
                break;
            }
        }
        let resources = found.ok_or_else(|| {
            BrowserError::InvalidResult("asset frame has no resource tree".into())
        })?;
        result.push((owner, resources));
    }
    Ok(result)
}
