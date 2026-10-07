//! HTML: the boot scripts come first, attribute values are read as the
//! browser decodes them, custom elements keep their attributes, and
//! integrity goes to the engine.

use demi_preview_rewrite::html::{HtmlOptions, rewrite_html};
use serde_json::json;

use super::{preview_origin, top_document};

const DOCUMENT: &str = "http://localhost:5173/";
const FRAGMENT: HtmlOptions = HtmlOptions { fragment: true, boot: None };

#[test]
fn a_document_starts_with_the_boot_scripts_whose_labels_cover_its_markup() {
    let context = top_document("http://localhost:5173");
    let boot = json!({ "url": DOCUMENT });
    let source = "<!doctype html><html><head><title>App</title><script src=\"https://cdn.example.com/lib.js\"></script></head><body></body></html>";
    let rewritten = rewrite_html(source, DOCUMENT, HtmlOptions { fragment: false, boot: Some(&boot) }, &context);
    let head = rewritten.find("<head>").unwrap() + "<head>".len();
    let scripts = &rewritten[head..];
    assert!(scripts.starts_with("<script src=\"/__demi/v1/client.js\"></script><script>globalThis.__proxyBoot="), "{rewritten}");
    let cdn = preview_origin("https://cdn.example.com", "http://localhost", true);
    let label = cdn.trim_start_matches("https://").split('.').next().unwrap().split("--").nth(1).unwrap();
    // The boot data names the label of the script's address, which the
    // document maps after the boot scripts are written.
    let boot_end = scripts.find("</script><script src=\"/__demi/page/runtime/").unwrap();
    assert!(scripts[..boot_end].contains(label), "{rewritten}");
    assert!(rewritten.contains(&format!("<script src=\"{cdn}/lib.js\">")), "{rewritten}");
}

#[test]
fn attribute_values_are_rewritten_as_the_browser_decodes_them() {
    let context = top_document("http://localhost:5173");
    let cdn = preview_origin("https://cdn.example.com", "http://localhost", true);
    let style = rewrite_html("<div style=\"--logo:url(&quot;https://cdn.example.com/logo.svg&quot;)\"></div>", DOCUMENT, FRAGMENT, &context);
    assert_eq!(style, format!("<div style=\"--logo:url(&quot;{cdn}/logo.svg&quot;)\"></div>"));
    let srcdoc = rewrite_html("<iframe srcdoc=\"&lt;img src=&quot;https://cdn.example.com/a.png&quot;&gt;\"></iframe>", DOCUMENT, FRAGMENT, &context);
    assert!(srcdoc.contains(&format!("{cdn}/a.png")), "{srcdoc}");
}

#[test]
fn a_custom_elements_attributes_are_its_own_data() {
    let context = top_document("http://localhost:5173");
    let source = "<vimeo-video src=\"https://vimeo.com/76979871\"></vimeo-video><img src=\"https://vimeo.com/poster.jpg\">";
    let rewritten = rewrite_html(source, DOCUMENT, FRAGMENT, &context);
    let vimeo = preview_origin("https://vimeo.com", "http://localhost", true);
    assert_eq!(
        rewritten,
        format!("<vimeo-video src=\"https://vimeo.com/76979871\"></vimeo-video><img src=\"{vimeo}/poster.jpg\">")
    );
}

#[test]
fn integrity_goes_to_the_engine_with_the_address() {
    let context = top_document("http://localhost:5173");
    let source = "<script src=\"https://cdn.example.com/lib.js\" integrity=\"sha384-abc\" crossorigin></script>";
    let rewritten = rewrite_html(source, DOCUMENT, FRAGMENT, &context);
    let cdn = preview_origin("https://cdn.example.com", "http://localhost", true);
    assert_eq!(rewritten, format!("<script src=\"{cdn}/lib.js?__demi_integrity=sha384%2Dabc\" crossorigin></script>"));
}

#[test]
fn script_text_that_is_not_javascript_stays_as_written() {
    let context = top_document("http://localhost:5173");
    let shader = "<script type=\"x-shader/x-fragment\">void main() { gl_FragColor = vec4(location); }</script>";
    assert_eq!(rewrite_html(shader, DOCUMENT, FRAGMENT, &context), shader);
    let broken = "<script>if (location {</script>";
    assert_eq!(rewrite_html(broken, DOCUMENT, FRAGMENT, &context), broken);
}
