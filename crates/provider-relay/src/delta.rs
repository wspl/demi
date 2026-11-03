//! A session's requests as what changed since its last.
//!
//! A request carries the whole transcript, so each tool round of a long
//! session would send it again. Both ends of a socket keep the last request
//! it carried: the borrower's [`Outbox`] sends
//! the system prompt and the tools only when they changed, and the items and
//! the blocks as how many of the last request's are kept, then the rest; the
//! lender's [`Inbox`] puts the request back together. When the last request
//! is not a prefix, as after a compaction, they cross whole. The two keep the
//! same last request because they keep it for the same socket, in the order
//! its messages cross.

use std::sync::Arc;

use demi_provider_common::{InferenceItem, InferenceRequest, RequestBlock, ToolDefinition};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::wire::{WireBlock, WireItem, WireRequest, WireTool};

/// A list as what changed since the last request's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Delta<T> {
    Whole { list: Vec<T> },
    /// The last request's first `kept`, then `appended`.
    After { kept: usize, appended: Vec<T> },
}

/// What one end keeps of the socket's last request.
#[derive(Default)]
struct Last {
    system_prompt: Option<String>,
    tools: Option<Arc<[ToolDefinition]>>,
    items: Option<Arc<[InferenceItem]>>,
    blocks: Option<Arc<[RequestBlock]>>,
}

/// The borrower's end.
#[derive(Default)]
pub(crate) struct Outbox(Last);

impl Outbox {
    /// `request` as it crosses, which it keeps as the last.
    pub(crate) fn send(&mut self, request: &InferenceRequest) -> WireRequest {
        let last = &mut self.0;
        let system_prompt = (last.system_prompt.as_deref() != Some(request.system_prompt.as_str()))
            .then(|| request.system_prompt.clone());
        let tools = match &last.tools {
            Some(tools) if Arc::ptr_eq(tools, &request.tools) || **tools == *request.tools => None,
            _ => Some(request.tools.iter().map(WireTool::from).collect()),
        };
        let items = changed(last.items.as_deref(), &request.items, |item| WireItem::from(item));
        let blocks = changed(last.blocks.as_deref(), &request.blocks, |block| WireBlock::from(block));
        last.system_prompt = Some(request.system_prompt.clone());
        last.tools = Some(request.tools.clone());
        last.items = Some(request.items.clone());
        last.blocks = Some(request.blocks.clone());
        WireRequest::of(request, system_prompt, tools, items, blocks)
    }
}

fn changed<T: PartialEq, W>(last: Option<&[T]>, now: &[T], wire: impl Fn(&T) -> W) -> Delta<W> {
    match last {
        Some(last) if last.len() <= now.len() && last == &now[..last.len()] => Delta::After {
            kept: last.len(),
            appended: now[last.len()..].iter().map(wire).collect(),
        },
        _ => Delta::Whole {
            list: now.iter().map(wire).collect(),
        },
    }
}

/// The lender's end.
#[derive(Default)]
pub(crate) struct Inbox(Last);

/// A request that does not follow the socket's last: the borrower broke the
/// wire's rules.
#[derive(Debug, thiserror::Error)]
#[error("the relayed request does not follow the session's last one: {0}")]
pub(crate) struct OutOfStep(&'static str);

impl Inbox {
    /// The request `wire` gives, which it keeps as the last.
    pub(crate) fn receive(
        &mut self,
        wire: WireRequest,
        cancel: CancellationToken,
    ) -> Result<InferenceRequest, OutOfStep> {
        let last = &mut self.0;
        let system_prompt = match wire.system_prompt.clone() {
            Some(prompt) => prompt,
            None => last.system_prompt.clone().ok_or(OutOfStep("the system prompt"))?,
        };
        let tools: Arc<[ToolDefinition]> = match wire.tools.clone() {
            Some(tools) => tools.into_iter().map(ToolDefinition::from).collect(),
            None => last.tools.clone().ok_or(OutOfStep("the tools"))?,
        };
        let items = joined(last.items.as_deref(), wire.items.clone(), InferenceItem::from)
            .ok_or(OutOfStep("the items"))?;
        let blocks = joined(last.blocks.as_deref(), wire.blocks.clone(), RequestBlock::from)
            .ok_or(OutOfStep("the blocks"))?;
        last.system_prompt = Some(system_prompt.clone());
        last.tools = Some(tools.clone());
        last.items = Some(items.clone());
        last.blocks = Some(blocks.clone());
        Ok(wire.into_request(system_prompt, tools, items, blocks, cancel))
    }
}

fn joined<T: Clone, W>(last: Option<&[T]>, delta: Delta<W>, own: impl Fn(W) -> T) -> Option<Arc<[T]>> {
    match delta {
        Delta::Whole { list } => Some(list.into_iter().map(own).collect()),
        Delta::After { kept, appended } => {
            let last = last?;
            let kept = last.get(..kept)?;
            Some(kept.iter().cloned().chain(appended.into_iter().map(own)).collect())
        }
    }
}
