//! Discover and call native page tools within the page's WebMCP permissions.

use chromiumoxide::{
    Page,
    cdp::{
        browser_protocol::{network::LoaderId, page::GetFrameTreeParams},
        js_protocol::runtime::EvaluateParams,
    },
};
use demi_command_service::InvocationContext;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    BrowserEnvironment, BrowserError, BrowserTab, Result,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::{
        BrowserOperation, Capability, INLINE_BYTES, WebmcpCallResult, WebmcpListResult, WebmcpTool,
    },
};

#[derive(Default)]
pub(super) struct State {
    document: Option<LoaderId>,
    key: Option<String>,
    tools: Option<ToolSet>,
}

struct ToolSet {
    handle: String,
    entries: Vec<WebmcpTool>,
}

/// Report native WebMCP availability in this document, including permissions.
pub(super) async fn capability(page: &Page) -> Result<Capability> {
    let available: bool = page.evaluate_expression("Boolean(document.modelContext && typeof document.modelContext.getTools === 'function' && typeof document.modelContext.executeTool === 'function')").await?
        .into_value().map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    Ok(Capability {
        id: "webmcp".into(),
        available,
        reason: (!available).then(|| {
            "this browser release or document does not expose document.modelContext".into()
        }),
        schema: available.then(|| json!({"help": "demi browser webmcp --help"})),
    })
}

/// Validate a discovered declaration before calling its native page tool.
pub(super) async fn execute(
    _context: &InvocationContext,
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
    let capability = operation.run(capability(&tab.page)).await?;
    if !capability.available {
        return Err(BrowserError::UnsupportedCapability(
            capability
                .reason
                .unwrap_or_else(|| "WebMCP is unavailable".into()),
        ));
    }
    let mut state = tab.state.webmcp.lock().await;
    let document = operation
        .run(async {
            Ok(tab
                .page
                .execute(GetFrameTreeParams {})
                .await?
                .result
                .frame_tree
                .frame
                .loader_id)
        })
        .await?;
    if state.document.as_ref() != Some(&document) {
        state.document = Some(document);
        state.key = None;
        state.tools = None;
    }
    {
        let key = match &state.key {
            Some(key) => key.clone(),
            None => super::handles::fresh("webmcp")?,
        };
        state.key = Some(key.clone());
        let script = format!(
            r#"(() => {{
            const key = {};
            if (globalThis[key]) return true;
            const context = document.modelContext;
            const state = {{version: 0, current: null, calls: new Map(), context}};
            Object.defineProperty(globalThis, key, {{value: state}});
            context.addEventListener('toolchange', () => {{ state.version++; state.current = null; }});
            return true;
        }})()"#,
            json!(key)
        );
        let installed: bool = operation
            .run(async {
                tab.page
                    .evaluate_expression(script)
                    .await?
                    .into_value()
                    .map_err(|error| BrowserError::InvalidResult(error.to_string()))
            })
            .await?;
        if !installed {
            return Err(BrowserError::InvalidResult(
                "WebMCP observer was not installed".into(),
            ));
        }
    }
    let key = state.key.as_ref().expect("observer installed").clone();
    match command {
        BrowserOperation::WebmcpList(_) => {
            let handle = super::handles::fresh("tools")?;
            let script = format!(
                r#"(async () => {{
                const state = globalThis[{}];
                const version = state.version;
                const tools = await state.context.getTools();
                if (version !== state.version) return {{status:'stale'}};
                const entries = tools.map(tool => ({{name:tool.name, description:tool.description, inputSchema:JSON.parse(tool.inputSchema)}}));
                const handle = state.current?.version === version ? state.current.handle : {};
                state.current = {{version, tools, handle}};
                return {{status:'ready', entries, handle}};
            }})()"#,
                json!(key),
                json!(handle)
            );
            let snapshot: Snapshot = operation
                .run(async {
                    tab.page
                        .evaluate_expression(
                            EvaluateParams::builder()
                                .expression(script)
                                .await_promise(true)
                                .return_by_value(true)
                                .build()
                                .map_err(BrowserError::Configuration)?,
                        )
                        .await?
                        .into_value()
                        .map_err(|error| BrowserError::InvalidResult(error.to_string()))
                })
                .await?;
            let Snapshot::Ready { entries, handle } = snapshot else {
                return Err(BrowserError::StaleTools);
            };
            for entry in &entries {
                jsonschema::options()
                    .offline()
                    .build(&entry.input_schema)
                    .map_err(|error| {
                        BrowserError::InvalidResult(format!("invalid tool input schema: {error}"))
                    })?;
                if let Some(schema) = &entry.output_schema {
                    jsonschema::options()
                        .offline()
                        .build(schema)
                        .map_err(|error| {
                            BrowserError::InvalidResult(format!(
                                "invalid tool output schema: {error}"
                            ))
                        })?;
                }
            }
            let result = super::output::value(WebmcpListResult {
                tools: handle.clone(),
                entries: entries.clone(),
                truncated: false,
            })?;
            if serde_json::to_vec(&result)
                .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
                .len()
                > INLINE_BYTES
            {
                return Err(BrowserError::ResultTooLarge);
            }
            state.tools = Some(ToolSet { handle, entries });
            Ok(result)
        }
        BrowserOperation::WebmcpCall(input) => {
            let tools = state
                .tools
                .as_ref()
                .filter(|tools| tools.handle == input.tools)
                .ok_or(BrowserError::StaleTools)?;
            let entries = &tools.entries;
            let mut matches = entries.iter().filter(|entry| entry.name == input.tool);
            let entry = matches.next().ok_or(BrowserError::TargetNotFound)?;
            if matches.next().is_some() {
                return Err(BrowserError::Ambiguous(
                    entries
                        .iter()
                        .filter(|entry| entry.name == input.tool)
                        .count(),
                ));
            }
            let arguments: Value = serde_json::from_str(&input.arguments)
                .map_err(|error| BrowserError::Configuration(error.to_string()))?;
            jsonschema::options()
                .offline()
                .build(&entry.input_schema)
                .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
                .validate(&arguments)
                .map_err(|error| BrowserError::Configuration(error.to_string()))?;
            let call = super::handles::fresh("toolcall")?;
            let script = format!(
                r#"(async () => {{
                const state = globalThis[{}];
                const set = state?.current;
                if (!set || set.handle !== {} || set.version !== state.version) return {{status:'stale'}};
                const tool = set.tools.find(tool => tool.name === {});
                if (!tool) return {{status:'stale'}};
                const controller = new AbortController();
                state.calls.set({}, controller);
                try {{
                    const result = await state.context.executeTool(tool, {}, {{signal:controller.signal}});
                    return {{status:'completed', result: result === null ? null : JSON.parse(result)}};
                }} finally {{ state.calls.delete({}); }}
            }})()"#,
                json!(key),
                json!(input.tools),
                json!(input.tool),
                json!(call),
                json!(arguments.to_string()),
                json!(call)
            );
            let work = operation
                .run(async {
                    operation.begin_input();
                    let reply: CallReply = tab
                        .page
                        .evaluate_expression(
                            EvaluateParams::builder()
                                .expression(script)
                                .await_promise(true)
                                .return_by_value(true)
                                .build()
                                .map_err(BrowserError::Configuration)?,
                        )
                        .await?
                        .into_value()
                        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
                    match reply {
                        CallReply::Stale => {
                            operation.input_not_delivered();
                            Err(BrowserError::StaleTools)
                        }
                        CallReply::Completed { result } => {
                            operation.complete_input();
                            if let Some(schema) = &entry.output_schema {
                                jsonschema::options()
                                    .offline()
                                    .build(schema)
                                    .map_err(|error| {
                                        BrowserError::InvalidResult(error.to_string())
                                    })?
                                    .validate(&result)
                                    .map_err(|error| {
                                        BrowserError::InvalidResult(error.to_string())
                                    })?;
                            }
                            super::output::value(WebmcpCallResult {
                                name: input.tool.clone(),
                                result,
                            })
                        }
                    }
                })
                .await;
            let script = format!(
                "(() => {{ const state = globalThis[{}]; const call = state?.calls.get({}); if (call) {{ call.abort(); state.calls.delete({}); }} return true; }})()",
                json!(key),
                json!(call),
                json!(call)
            );
            let cleanup =
                tokio::time::timeout(CONTROL_TIMEOUT, tab.page.evaluate_expression(script))
                    .await
                    .map_err(|_| BrowserError::Timeout)
                    .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
            let cleanup = match cleanup {
                // Destroyed documents cannot retain a pending tool execution.
                Err(
                    BrowserError::Closed | BrowserError::Connection(_) | BrowserError::TabNotFound,
                ) => Ok(()),
                result => result,
            };
            after_cleanup(work, cleanup)
                .map_err(|error| operation.failure(error, tab.id().as_str(), None))
        }
        _ => unreachable!("WebMCP dispatch accepts only list/call"),
    }
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "lowercase", deny_unknown_fields)]
enum Snapshot {
    Stale,
    Ready {
        entries: Vec<WebmcpTool>,
        handle: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "lowercase", deny_unknown_fields)]
enum CallReply {
    Stale,
    Completed { result: Value },
}
