//! JavaScript: what the page reads as its own location, top and eval goes
//! through the runtime, module addresses map, and every other byte stays.

use demi_preview_rewrite::boot::WorkerScript;
use demi_preview_rewrite::javascript::{ScriptOptions, SourceMapMode, rewrite_javascript};

use super::{RUNTIME, preview_origin, top_document};

const URL: &str = "http://localhost:5173/src/main.js";

fn options(worker: Option<WorkerScript<'static>>) -> ScriptOptions<'static> {
    ScriptOptions { filename: URL, base: None, module_url: None, source_map: SourceMapMode::Omitted, handler: false, worker }
}

fn rewrite(source: &str) -> String {
    rewrite_javascript(source, options(None), &top_document("http://localhost:5173")).unwrap()
}

#[test]
fn code_that_touches_nothing_the_runtime_takes_over_keeps_every_byte() {
    let source = "// counter\nconst add = (a, b) => a + b;\nexport default { add, total: add(1, 2) };\n";
    assert_eq!(rewrite(source), source);
}

#[test]
fn the_global_location_goes_through_the_runtime_and_a_local_one_does_not() {
    let rewritten = rewrite("location.href = '/next';");
    assert!(rewritten.contains("__proxyLocation"), "{rewritten}");
    let local = "function go(location) { return location.pathname; }";
    let rewritten = rewrite(local);
    assert!(!rewritten.contains("__proxyLocation;") && !rewritten.contains("__proxyLocation."), "{rewritten}");
}

#[test]
fn the_global_eval_rewrites_its_code_and_an_unparsable_script_is_an_error() {
    let rewritten = rewrite("eval(code);");
    assert_eq!(rewritten, "eval(__proxyRewriteJavaScript(code));");
    let context = top_document("http://localhost:5173");
    assert!(rewrite_javascript("let = = 1;", options(None), &context).is_err());
}

#[test]
fn absolute_imports_map_and_bare_imports_stay() {
    let esm = preview_origin("https://esm.sh", "http://localhost", true);
    let rewritten = rewrite("import React from 'https://esm.sh/react';\nimport { ref } from 'vue';\n");
    assert_eq!(rewritten, format!("import React from \"{esm}/react\";\nimport {{ ref }} from 'vue';\n"));
}

#[test]
fn a_workers_own_script_starts_with_the_runtime() {
    let context = top_document("http://localhost:5173");
    let own = preview_origin("http://localhost:5173", "http://localhost", false);
    let worker = WorkerScript { module: false, referrer: "http://localhost:5173/" };
    let classic = rewrite_javascript("postMessage(1);", options(Some(worker)), &context).unwrap();
    let (first, rest) = classic.split_once('\n').unwrap();
    assert!(first.starts_with("globalThis.__proxyBoot="), "{first}");
    assert!(first.ends_with(&format!("importScripts(\"{own}{RUNTIME}\");")), "{first}");
    assert_eq!(rest, "postMessage(1);");
    // A module's imports run before its body, so it imports the runtime.
    let worker = WorkerScript { module: true, ..worker };
    let module = rewrite_javascript("postMessage(1);", options(Some(worker)), &context).unwrap();
    let (first, _) = module.split_once('\n').unwrap();
    assert!(first.starts_with("import \"data:text/javascript,"), "{first}");
    assert!(first.ends_with(&format!("import \"{own}{RUNTIME}\";")), "{first}");
}
