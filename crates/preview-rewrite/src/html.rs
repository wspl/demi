//! HTML rewriting, streaming over the markup: addresses, inline scripts and styles, event
//! handlers and srcdoc documents, and the boot scripts that start the runtime first.

use std::cell::RefCell;
use std::rc::Rc;

use lol_html::html_content::{ContentType, Element, TextChunk};
use lol_html::{HandlerResult, Settings, element, rewrite_str, text};
use serde_json::json;
use url::Url;

use crate::address::{Context, Role, address_role, map_url, with_parameter};
use crate::attributes::{rewrite_import_map, rewrite_refresh, rewrite_resource_attribute};
use crate::boot::boot_data;
use crate::css::rewrite_css;
use crate::javascript::{ScriptOptions, SourceMapMode, rewrite_javascript};

const SCRIPT_TYPES: [&str; 4] = ["", "module", "text/javascript", "application/javascript"];

#[derive(Clone, Copy, Debug)]
pub struct HtmlOptions<'o> {
    /// Markup inserted into an existing document (innerHTML, document.write): no boot scripts.
    pub fragment: bool,
    /// The boot data for a document whose runtime starts here. Its labels are completed with
    /// every address the document maps.
    pub boot: Option<&'o serde_json::Value>,
}

#[derive(Clone, Copy, PartialEq)]
enum Text {
    Untouched,
    Script,
    ImportMap,
}

/// What the streaming handlers share: the document's base, whether the runtime is in, and
/// the text of the script or style element being read.
struct State {
    base: String,
    base_set: bool,
    booted: bool,
    script: Text,
    buffer: String,
}

/// Where the boot data goes until the whole document is rewritten: the runtime reads back the
/// addresses the document's markup maps (a script's `src`) before any other way could tell it
/// their labels.
const BOOT_PLACEHOLDER: &str = "__demi_boot_data__";

/// The scripts that start a preview document before any page script: the preview's client
/// script, the boot data, then the runtime.
fn boot_scripts(context: &Context) -> String {
    format!("<script src=\"{}\"></script><script>globalThis.__proxyBoot={BOOT_PLACEHOLDER}</script><script src=\"{}\"></script>", context.client, context.runtime)
}

fn completed_boot(boot: &serde_json::Value, context: &Context) -> String {
    let mut boot = boot.clone();
    boot["labels"] = json!(context.labels());
    boot.to_string().replace('<', "\\u003c")
}

fn script(source: &str, filename: &str, base: &str, handler: bool, context: &Context) -> Option<String> {
    let options = ScriptOptions { filename, base: Some(base), module_url: None, source_map: SourceMapMode::Omitted, handler, worker: None };
    // Code that does not parse is left for the browser to report.
    rewrite_javascript(source, options, context).ok()
}

fn javascript_url(value: &str) -> bool {
    value.trim_start().get(..11).is_some_and(|scheme| scheme.eq_ignore_ascii_case("javascript:"))
}

/// An attribute's value as the browser reads it: lol_html gives the source text, entities
/// and all, and an address or a style is parsed only after they are decoded.
fn read_attribute(element: &Element, name: &str) -> Option<String> {
    element.get_attribute(name).map(|value| html_escape::decode_html_entities(&value).into_owned())
}

/// Set an attribute from its decoded value: lol_html escapes `"` but not `&`.
fn write_attribute(element: &mut Element, name: &str, value: &str) -> HandlerResult {
    element.set_attribute(name, &value.replace('&', "&amp;"))?;
    Ok(())
}

fn rewrite_element(element: &mut Element, state: &RefCell<State>, document_url: &str, options: HtmlOptions, context: &Context) -> HandlerResult {
    let tag = element.tag_name();
    {
        let mut state = state.borrow_mut();
        if options.boot.is_some() && !state.booted && !options.fragment {
            if tag == "head" {
                element.prepend(&boot_scripts(context), ContentType::Html);
                state.booted = true;
            } else if tag != "html" {
                element.before(&boot_scripts(context), ContentType::Html);
                state.booted = true;
            }
        }
        // The first <base href> sets the base for every address after it.
        if tag == "base" && !state.base_set
            && let Some(href) = read_attribute(element, "href")
            && let Ok(base) = Url::parse(document_url).and_then(|url| url.join(href.trim()))
        {
            state.base = base.into();
            state.base_set = true;
        }
    }
    let base = state.borrow().base.clone();
    let script_type = read_attribute(element, "type");
    let rel = read_attribute(element, "rel");
    let target = read_attribute(element, "target");
    let attributes: Vec<(String, String)> = element.attributes().iter().map(|attribute| (attribute.name(), html_escape::decode_html_entities(&attribute.value()).into_owned())).collect();
    for (name, value) in attributes {
        let rewritten = if name.starts_with("on") {
            script(&value, "inline-handler.js", &base, true, context)
        } else if name == "srcdoc" {
            Some(rewrite_html(&value, &base, HtmlOptions { fragment: false, boot: Some(&boot_data(context, "about:srcdoc", &base, &base)) }, context))
        } else if javascript_url(&value) && matches!(name.as_str(), "src" | "href" | "action" | "formaction") {
            let code = &value[value.find(':').map_or(0, |index| index + 1)..];
            script(code, "javascript-url.js", &base, false, context).map(|code| format!("javascript:{code}"))
        } else if tag.contains('-') && name != "style" {
            // A custom element's address attributes are its own data: its script reads them
            // (in attributeChangedCallback too) and maps what it loads itself.
            None
        } else {
            // A <base href> resolves against the document's URL; a foreign one is mapped, so
            // addresses relative to it lead to that origin's preview address.
            let base_href = tag == "base" && name == "href";
            let attribute_base = if base_href { document_url } else { &base };
            let role = if base_href { Role::Resource } else { address_role(&tag, &name, script_type.as_deref(), rel.as_deref(), target.as_deref()) };
            Some(rewrite_resource_attribute(&name, &value, attribute_base, false, role, context))
        };
        if let Some(rewritten) = rewritten.filter(|rewritten| *rewritten != value) {
            write_attribute(element, &name, &rewritten)?;
        }
    }
    // The engine checks integrity against the upstream bytes; the browser would check the
    // rewritten ones.
    let address_attribute = match tag.as_str() {
        "script" => Some("src"),
        "link" => Some("href"),
        _ => None,
    };
    if let Some(attribute) = address_attribute
        && let Some(integrity) = read_attribute(element, "integrity")
        && let Some(address) = read_attribute(element, attribute)
        && let Some(absolute) = map_url(address.trim(), Some(&base), Role::Resource, context)
    {
        write_attribute(element, attribute, &with_parameter(&absolute, "integrity", &integrity))?;
        element.remove_attribute("integrity");
    }
    if tag == "meta"
        && read_attribute(element, "http-equiv").is_some_and(|value| value.eq_ignore_ascii_case("refresh"))
        && let Some(content) = read_attribute(element, "content")
    {
        write_attribute(element, "content", &rewrite_refresh(&content, &base, context))?;
    }
    if tag == "script" && !element.has_attribute("src") {
        let kind = script_type.as_deref().map(str::trim).unwrap_or_default().to_ascii_lowercase();
        state.borrow_mut().script = if SCRIPT_TYPES.contains(&kind.as_str()) { Text::Script } else if kind == "importmap" { Text::ImportMap } else { Text::Untouched };
    }
    Ok(())
}

/// Collect an element's text across chunks and replace it, rewritten, at its last chunk.
fn rewrite_text(chunk: &mut TextChunk, state: &RefCell<State>, rewrite: impl FnOnce(&str, &str) -> String) {
    let mut state = state.borrow_mut();
    state.buffer.push_str(chunk.as_str());
    if !chunk.last_in_text_node() {
        chunk.remove();
        return;
    }
    let text = std::mem::take(&mut state.buffer);
    let output = if text.trim().is_empty() { text } else { rewrite(&text, &state.base) };
    chunk.replace(&output, ContentType::Html);
}

/// Rewrite a document or fragment whose addresses resolve against `document_url`.
pub fn rewrite_html(source: &str, document_url: &str, options: HtmlOptions, context: &Context) -> String {
    let state = Rc::new(RefCell::new(State { base: document_url.to_owned(), base_set: false, booted: false, script: Text::Untouched, buffer: String::new() }));
    let (elements, scripts, styles) = (state.clone(), state.clone(), state.clone());
    let settings = Settings::new()
        .append_element_content_handler(element!("*", move |element| rewrite_element(element, &elements, document_url, options, context)))
        .append_element_content_handler(text!("script", move |chunk| {
            let kind = scripts.borrow().script;
            match kind {
                Text::Untouched => {}
                Text::Script => rewrite_text(chunk, &scripts, |text, base| script(text, "inline.js", base, false, context).unwrap_or_else(|| text.to_owned())),
                Text::ImportMap => rewrite_text(chunk, &scripts, |text, base| rewrite_import_map(text, base, context)),
            }
            Ok(())
        }))
        .append_element_content_handler(text!("style", move |chunk| {
            rewrite_text(chunk, &styles, |text, base| rewrite_css(text, base, false, context));
            Ok(())
        }));
    let mut output = rewrite_str(source, settings).unwrap_or_else(|_| source.to_owned());
    let Some(boot) = options.boot.filter(|_| !options.fragment) else { return output };
    if !state.borrow().booted {
        output.insert_str(0, &boot_scripts(context));
    }
    // The first one is the boot scripts': they come before the page's own markup.
    output.replacen(BOOT_PLACEHOLDER, &completed_boot(boot, context), 1)
}
