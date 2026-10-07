//! Maps for debuggers, composed only when a debugger asks (`preview.md`
//! § Rewriting): the rewriter's map leads from the rewritten script to the
//! upstream script, and the upstream's own map (inline or external) leads on
//! to the authored sources. Without an upstream map, the result leads to the
//! upstream script.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use demi_command_package_browser_protocol::preview::PreviewClient;
use demi_preview_rewrite::address::{Context, Environment};
use demi_preview_rewrite::boot::WorkerScript;
use demi_preview_rewrite::javascript::{ScriptOptions, SourceMapMode, rewrite_javascript_with_map};
use oxc_sourcemap::{SourceMap, SourceMapBuilder};
use url::Url;

use crate::engine::Engine;

type Error = Box<dyn std::error::Error + Send + Sync>;

/// The `sourceMappingURL` a script names, if any: its last such comment.
fn named_map(source: &str) -> Option<&str> {
    source.lines().rev().find_map(|line| {
        let trimmed = line.trim();
        let comment = trimmed.strip_prefix("//#").or_else(|| trimmed.strip_prefix("//@"))?;
        comment.trim_start().strip_prefix("sourceMappingURL=").map(str::trim)
    })
}

/// The upstream map's JSON and the URL its sources resolve against.
async fn upstream_map(
    engine: &Engine,
    client: &PreviewClient,
    receiver: &Environment,
    script: &Url,
    source: &str,
) -> Option<(String, Url)> {
    let named = named_map(source)?;
    if let Some(data) = named.strip_prefix("data:") {
        let (metadata, payload) = data.split_once(',')?;
        let json = if metadata.ends_with(";base64") {
            String::from_utf8(STANDARD.decode(payload).ok()?).ok()?
        } else {
            percent_encoding::percent_decode_str(payload).decode_utf8().ok()?.into_owned()
        };
        return Some((json, script.clone()));
    }
    let address = script.join(named).ok()?;
    // A published script often names a map that was never published.
    let json = engine.fetch_text(client, receiver, &address).await.ok()?;
    Some((json, address))
}

pub(crate) async fn compose(
    engine: &Engine,
    client: &PreviewClient,
    script: &Url,
    context: &Context,
    worker: Option<WorkerScript<'_>>,
) -> Result<String, Error> {
    let receiver = &context.document;
    let source = engine.fetch_text(client, receiver, script).await?;
    let options = ScriptOptions {
        filename: script.as_str(),
        base: None,
        module_url: None,
        source_map: SourceMapMode::Returned,
        handler: false,
        worker,
    };
    let rewritten = rewrite_javascript_with_map(&source, options, context).map_err(|error| error.0)?;
    let own = rewritten.map.ok_or("the rewriter returned no map")?;
    let Some((upstream_json, map_address)) = upstream_map(engine, client, receiver, script, &source).await else {
        return Ok(own);
    };
    let own_map = SourceMap::from_json_string(&own)?;
    let upstream = SourceMap::from_json_string(&upstream_json)?;
    let root = serde_json::from_str::<serde_json::Value>(&upstream_json)
        .ok()
        .and_then(|value| value.get("sourceRoot").and_then(|root| root.as_str()).map(ToOwned::to_owned))
        .unwrap_or_default();
    let base = map_address.join(&root).unwrap_or(map_address);
    let sources: Vec<String> = upstream
        .get_sources()
        .map(|source| base.join(source).map_or_else(|_| source.to_owned(), Into::into))
        .collect();
    let contents: Vec<String> = upstream
        .get_source_contents()
        .map(|content| content.unwrap_or_default().to_owned())
        .collect();
    let names: Vec<String> = (0..).map_while(|id| upstream.get_name(id).map(ToOwned::to_owned)).collect();
    let table = upstream.generate_lookup_table();
    let mut builder = SourceMapBuilder::default();
    let source_ids: Vec<u32> = sources
        .iter()
        .zip(&contents)
        .map(|(source, content)| builder.add_source_and_content(source, content))
        .collect();
    let name_ids: Vec<u32> = names.iter().map(|name| builder.add_name(name)).collect();
    for token in own_map.get_tokens() {
        let Some(original) = upstream.lookup_token(&table, token.get_src_line(), token.get_src_col()) else {
            continue;
        };
        let Some(source) = original.get_source_id().and_then(|id| source_ids.get(id as usize).copied()) else {
            continue;
        };
        let name = original.get_name_id().and_then(|id| name_ids.get(id as usize).copied());
        builder.add_token(
            token.get_dst_line(),
            token.get_dst_col(),
            original.get_src_line(),
            original.get_src_col(),
            Some(source),
            name,
        );
    }
    Ok(builder.into_sourcemap().to_json_string())
}
