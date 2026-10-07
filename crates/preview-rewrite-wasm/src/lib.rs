//! The rewriter for the page's runtime (`crates-and-packages.md`
//! § preview-rewrite-wasm): code and markup the page creates itself (eval,
//! innerHTML, srcdoc, data: and blob: scripts) and the addresses it sets. It runs the same
//! code as the engine on the Host.

use demi_preview_rewrite::address::{self, Context, Environment, Role};
use demi_preview_rewrite::attributes::{self, REMOVED_CONTENT_POLICIES};
use demi_preview_rewrite::css::rewrite_stylesheet;
use demi_preview_rewrite::boot::{WorkerScript, boot_data};
use demi_preview_rewrite::html::{HtmlOptions, rewrite_html};
use demi_preview_rewrite::javascript::{FunctionKind, ScriptOptions, SourceMapMode, rewrite_function, rewrite_javascript_with_map};
use serde_json::json;
use wasm_bindgen::prelude::*;

fn role(name: &str) -> Role {
    match name {
        "child" => Role::Child,
        "module" => Role::Module,
        "navigation" => Role::Navigation,
        "top-navigation" => Role::TopNavigation,
        _ => Role::Resource,
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Child => "child",
        Role::Module => "module",
        Role::Navigation => "navigation",
        Role::TopNavigation => "top-navigation",
        Role::Resource => "resource",
    }
}

/// The address and rewriting operations of one document or worker.
#[wasm_bindgen]
pub struct Rewriter {
    context: Context,
}

#[wasm_bindgen]
impl Rewriter {
    /// `context` is the boot data (`{ domain, namespace, host, boot, runtime, document, labels }`)
    /// with the runtime's `topLevel`.
    #[wasm_bindgen(constructor)]
    pub fn new(context: &str) -> Result<Rewriter, JsError> {
        let value: serde_json::Value = serde_json::from_str(context)?;
        let rewriter = Rewriter { context: serde_json::from_value(value.clone())? };
        if let Some(labels) = value.get("labels") {
            rewriter.context.learn(serde_json::from_value(labels.clone())?);
        }
        Ok(rewriter)
    }

    #[wasm_bindgen(js_name = mapUrl)]
    pub fn map_url(&self, value: &str, base: Option<String>, role_name: &str) -> Option<String> {
        address::map_url(value, base.as_deref(), role(role_name), &self.context)
    }

    #[wasm_bindgen(js_name = convertWrittenUrl)]
    pub fn convert_written_url(&self, text: &str, base: &str, reflect: bool, role_name: &str) -> String {
        address::convert_written_url(text, base, reflect, role(role_name), &self.context)
    }

    #[wasm_bindgen(js_name = mapModuleSpecifier)]
    pub fn map_module_specifier(&self, specifier: &str, base: Option<String>) -> String {
        address::map_module_specifier(specifier, base.as_deref(), &self.context)
    }

    /// The logical address behind a preview address of a known label; any other URL unchanged.
    #[wasm_bindgen(js_name = logicalUrl)]
    pub fn logical_url(&self, value: &str) -> Option<String> {
        address::logical_url(value, &self.context)
    }

    #[wasm_bindgen(js_name = writtenUrl)]
    pub fn written_url(&self, text: &str) -> String {
        address::written_url(text, &self.context)
    }

    #[wasm_bindgen(js_name = isPreviewUrl)]
    pub fn is_preview_url(&self, value: &str) -> bool {
        self.context.is_preview_url(value)
    }

    /// `{ label, target, bootstrap }` as JSON, or nothing.
    #[wasm_bindgen(js_name = parsePreviewUrl)]
    pub fn parse_preview_url(&self, value: &str) -> Option<String> {
        let parsed = self.context.parse_preview_url(value)?;
        Some(json!({ "label": parsed.label, "target": parsed.target, "bootstrap": parsed.bootstrap }).to_string())
    }

    /// The environment of a label this rewriter has met, as JSON.
    #[wasm_bindgen(js_name = environmentOf)]
    pub fn environment_of(&self, label: &str) -> Option<String> {
        self.context.environment_of(label).map(|environment| json!(environment).to_string())
    }

    /// Every label met so far and its environment, as JSON.
    pub fn labels(&self) -> String {
        json!(self.context.labels()).to_string()
    }

    /// Labels met elsewhere, as JSON: `{ label: environment }`.
    pub fn learn(&self, labels: &str) -> Result<(), JsError> {
        self.context.learn(serde_json::from_str(labels)?);
        Ok(())
    }

    /// Boot data for a document that starts in this one's context (a srcdoc document), as JSON.
    #[wasm_bindgen(js_name = bootData)]
    pub fn boot_data(&self, url: &str, base: &str, referrer: &str) -> String {
        boot_data(&self.context, url, base, referrer).to_string()
    }

    /// `source_map` is `"omitted"`, `"inline"`, `"returned"` or the URL the map is served at.
    /// Returns `{ code, map }`.
    #[wasm_bindgen(js_name = rewriteJavaScript)]
    pub fn rewrite_javascript(&self, source: &str, filename: &str, base: Option<String>, module_url: Option<String>, handler: bool, source_map: &str) -> Result<String, JsError> {
        let source_map = match source_map {
            "omitted" => SourceMapMode::Omitted,
            "inline" => SourceMapMode::Inline,
            "returned" => SourceMapMode::Returned,
            url => SourceMapMode::External(url),
        };
        let options = ScriptOptions { filename, base: base.as_deref(), module_url: module_url.as_deref(), source_map, handler, worker: None };
        let rewritten = rewrite_javascript_with_map(source, options, &self.context).map_err(|error| JsError::new(&error.0))?;
        Ok(json!({ "code": rewritten.code, "map": rewritten.map }).to_string())
    }

    /// A worker's script from a `blob:` or `data:` address (`filename`), which the engine never
    /// sees: rewritten, and started with the worker's runtime. `referrer` is the starting
    /// document's address.
    #[wasm_bindgen(js_name = rewriteWorker)]
    pub fn rewrite_worker(&self, source: &str, filename: &str, module: bool, referrer: &str) -> Result<String, JsError> {
        let worker = WorkerScript { module, referrer };
        let module_url = module.then_some(filename);
        let options = ScriptOptions { filename, base: Some(filename), module_url, source_map: SourceMapMode::Omitted, handler: false, worker: Some(worker) };
        Ok(rewrite_javascript_with_map(source, options, &self.context).map_err(|error| JsError::new(&error.0))?.code)
    }

    /// `kind` is the constructor's name: Function, AsyncFunction, GeneratorFunction or
    /// AsyncGeneratorFunction. Returns `[parameters, body]`.
    #[wasm_bindgen(js_name = rewriteFunction)]
    pub fn rewrite_function(&self, parameters: &str, body: &str, kind: &str) -> Result<Vec<String>, JsError> {
        let kind = match kind {
            "AsyncFunction" => FunctionKind::Async,
            "GeneratorFunction" => FunctionKind::Generator,
            "AsyncGeneratorFunction" => FunctionKind::AsyncGenerator,
            _ => FunctionKind::Plain,
        };
        let (parameters, body) = rewrite_function(parameters, body, kind, &self.context).map_err(|error| JsError::new(&error.0))?;
        Ok(vec![parameters, body])
    }

    #[wasm_bindgen(js_name = rewriteHtml)]
    pub fn rewrite_html(&self, source: &str, document_url: &str, fragment: bool, boot: Option<String>) -> Result<String, JsError> {
        let boot: Option<serde_json::Value> = boot.as_deref().map(serde_json::from_str).transpose()?;
        Ok(rewrite_html(source, document_url, HtmlOptions { fragment, boot: boot.as_ref() }, &self.context))
    }

    /// CSS whose addresses resolve against `base` and whose selectors match the document at
    /// `document_base`.
    #[wasm_bindgen(js_name = rewriteCss)]
    pub fn rewrite_css(&self, source: &str, base: &str, document_base: &str, reflect: bool) -> String {
        rewrite_stylesheet(source, base, document_base, reflect, &self.context)
    }

    #[wasm_bindgen(js_name = rewriteResourceAttribute)]
    pub fn rewrite_resource_attribute(&self, name: &str, value: &str, base: &str, reflect: bool, role_name: &str) -> String {
        attributes::rewrite_resource_attribute(name, value, base, reflect, role(role_name), &self.context)
    }

    #[wasm_bindgen(js_name = rewriteSrcset)]
    pub fn rewrite_srcset(&self, value: &str, base: &str, reflect: bool) -> String {
        attributes::rewrite_srcset(value, base, reflect, &self.context)
    }

    #[wasm_bindgen(js_name = rewriteImportMap)]
    pub fn rewrite_import_map(&self, source: &str, base: &str) -> String {
        attributes::rewrite_import_map(source, base, &self.context)
    }

    #[wasm_bindgen(js_name = rewriteRefresh)]
    pub fn rewrite_refresh(&self, value: &str, base: &str) -> String {
        attributes::rewrite_refresh(value, base, &self.context)
    }
}

#[wasm_bindgen(js_name = addressRole)]
pub fn address_role(tag: &str, attribute: &str, script_type: Option<String>, rel: Option<String>, target: Option<String>) -> String {
    role_name(address::address_role(tag, attribute, script_type.as_deref(), rel.as_deref(), target.as_deref())).into()
}

/// A preview address carrying an engine parameter (`integrity`, `tainted`).
#[wasm_bindgen(js_name = withParameter)]
pub fn with_parameter(address: &str, name: &str, value: &str) -> String {
    address::with_parameter(address, name, value)
}

/// The label of an environment given as JSON `{ origin, top, cross }`.
#[wasm_bindgen]
pub fn label(namespace: &str, host: &str, environment: &str) -> Result<String, JsError> {
    let environment: Environment = serde_json::from_str(environment)?;
    Ok(address::label(namespace, host, &environment))
}

#[wasm_bindgen(js_name = removedContentPolicies)]
pub fn removed_content_policies() -> Vec<String> {
    REMOVED_CONTENT_POLICIES.iter().map(|&name| name.into()).collect()
}

/// The attributes whose value is one address.
#[wasm_bindgen(js_name = urlAttributes)]
pub fn url_attributes() -> Vec<String> {
    attributes::URL_ATTRIBUTES.iter().map(|name| (*name).to_owned()).collect()
}
