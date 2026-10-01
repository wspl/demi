//! The `cdp` commands (`browser.md` § CDP commands and events): methods and
//! events of the pinned protocol on the calling agent's own debugging
//! connection to a tab, outside the denied domains and methods.

use chromiumoxide::cdp::browser_protocol::target::GetTargetInfoReturns;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use demi_browser_driver::operation::{BrowserError, Result, after_cleanup};
use demi_browser_tabs::{debug::Query, environment::BrowserEnvironment, tab::BrowserTab};
use demi_command_sdk::InvocationContext;

use crate::protocol::{
    BrowserErrorCode, BrowserOperation, Capability, CdpDetachResult, CdpSendResult, CdpTarget,
    CdpTargetsResult, DEFAULT_NODES,
};
use crate::{catalog::catalog, sessions};

const DENIED_DOMAINS: &[&str] = &[
    "Target",
    "Browser",
    "SystemInfo",
    "Tethering",
    "HeadlessExperimental",
];
const DENIED_METHODS: &[&str] = &["Page.close", "Page.crash", "Page.setDownloadBehavior"];

/// Expose the design's denied list without copying the pinned method catalog.
pub(crate) fn capability() -> Capability {
    Capability {
        id: "cdp".into(),
        available: true,
        reason: None,
        schema: Some(
            json!({"deniedDomains": DENIED_DOMAINS, "deniedMethods": DENIED_METHODS, "help": "demi browser cdp --help"}),
        ),
    }
}

/// Execute only pinned tab/child methods; never accept a caller session identifier.
pub async fn execute(
    context: &InvocationContext,
    _environment: &BrowserEnvironment,
    tab: Option<&BrowserTab>,
    command: &BrowserOperation,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    let tab = tab.ok_or(BrowserError::TabNotFound)?;
    let operation = tab.operation(cancel, deadline);
    let _session = tab.state().gate.try_checkout().ok_or(BrowserError::Busy)?;
    let owner = demi_browser_driver::operation::agent(context)?;
    let debug = tab.state().debug.owner(|| sessions::start(tab));
    if matches!(command, BrowserOperation::CdpDetach(_)) {
        debug.detach(owner).await?;
        return demi_browser_driver::output::value(CdpDetachResult {
            detached: tab.id().clone(),
        });
    }
    let parameters = if let BrowserOperation::CdpSend(input) = command {
        admit(&input.method)?;
        let params: Value = serde_json::from_str(&input.params)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        catalog()?
            .validate(&input.method, "params", &params)
            .map_err(|error| BrowserError::Configuration(error.to_string()))?;
        Some(params)
    } else {
        None
    };
    let connection = operation.run(debug.connect(owner)).await?;
    match command {
        BrowserOperation::CdpTargets(input) => {
            // A round trip flushes attachment events already queued by Chrome.
            operation
                .run(connection.send("Runtime.getIsolateId", json!({}), "main"))
                .await?;
            let targets = operation.run(connection.targets()).await?;
            let mut rows = Vec::with_capacity(targets.len());
            for id in targets {
                let value = match operation
                    .run(connection.send("Target.getTargetInfo", json!({}), &id))
                    .await
                {
                    Ok(value) => value,
                    // A child may close after the target snapshot was taken.
                    Err(BrowserError::TargetNotFound) => continue,
                    Err(error) => return Err(error),
                };
                let target: GetTargetInfoReturns =
                    serde_json::from_value(value).map_err(|error| {
                        BrowserError::Cdp(chromiumoxide::error::CdpError::msg(error.to_string()))
                    })?;
                rows.push(CdpTarget {
                    id,
                    kind: target.target_info.r#type,
                    url: target.target_info.url,
                });
            }
            rows.sort_by(|left, right| left.id.cmp(&right.id));
            let offset = input.offset.unwrap_or(0);
            let limit = input.limit.unwrap_or(DEFAULT_NODES);
            let count = rows.len();
            let targets = rows.into_iter().skip(offset).take(limit).collect();
            demi_browser_driver::output::value(CdpTargetsResult {
                targets,
                truncated: offset.saturating_add(limit) < count,
            })
        }
        BrowserOperation::CdpSend(input) => {
            let params = parameters.expect("send parameters were validated before attachment");
            let result = operation
                .run(connection.send(
                    &input.method,
                    params,
                    input.target.as_deref().unwrap_or("main"),
                ))
                .await;
            let sent = |result| CdpSendResult {
                method: input.method.clone(),
                result,
            };
            if result.as_ref().is_err_and(|error| {
                matches!(
                    error.code(),
                    BrowserErrorCode::Cancelled | BrowserErrorCode::Timeout
                )
            }) {
                let cleanup = debug.detach(owner).await;
                return after_cleanup(result.map(sent), cleanup)
                    .and_then(demi_browser_driver::output::value);
            }
            demi_browser_driver::output::value(sent(result?))
        }
        BrowserOperation::CdpEvents(input) => {
            if let Some(methods) = &input.method {
                for method in methods {
                    catalog()?.schema(method, "event")?;
                }
            }
            let target = input.target.as_deref().unwrap_or("main");
            if !operation
                .run(connection.targets())
                .await?
                .iter()
                .any(|known| known == target)
            {
                return Err(BrowserError::TargetNotFound);
            }
            loop {
                // Watched before the read, so an event recorded after it wakes the wait.
                let mut recorded = debug.recorded();
                let page = operation
                    .run(debug.events(Query {
                        after: input.after.clone(),
                        limit: input.limit.unwrap_or(DEFAULT_NODES),
                        methods: input.method.clone(),
                        target: target.to_owned(),
                    }))
                    .await?;
                if !page.wait || input.timeout.is_none() || tokio::time::Instant::now() >= deadline
                {
                    return demi_browser_driver::output::value(page.result);
                }
                let waited = operation
                    .run(async { recorded.changed().await.map_err(|_| BrowserError::Closed) })
                    .await;
                match waited {
                    Ok(()) => {}
                    Err(error) if context.cancellation.is_cancelled() => {
                        return after_cleanup(Err(error), debug.detach(owner).await);
                    }
                    Err(BrowserError::Timeout) => {
                        return demi_browser_driver::output::value(page.result);
                    }
                    Err(BrowserError::Cancelled) if tokio::time::Instant::now() >= deadline => {
                        return demi_browser_driver::output::value(page.result);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        _ => unreachable!("CDP detach was handled before targets/send/events"),
    }
}

fn admit(method: &str) -> Result<()> {
    let (domain, _) = method
        .split_once('.')
        .ok_or_else(|| BrowserError::Configuration("CDP method must be Domain.method".into()))?;
    if DENIED_DOMAINS.contains(&domain) || DENIED_METHODS.contains(&method) {
        return Err(BrowserError::CdpMethodDenied(method.into()));
    }
    catalog()?.schema(method, "params")?;
    Ok(())
}
