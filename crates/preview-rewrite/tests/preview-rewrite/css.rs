//! CSS: absolute resource addresses map, and attribute selectors naming
//! another origin match the preview addresses the DOM holds.

use demi_preview_rewrite::css::rewrite_css;

use super::{preview_origin, top_document};

const BASE: &str = "http://localhost:5173/";

#[test]
fn absolute_resource_addresses_map_and_every_other_byte_stays() {
    let context = top_document("http://localhost:5173");
    let cdn = preview_origin("https://cdn.example.com", "http://localhost", true);
    let source = "@import \"https://cdn.example.com/base.css\";\n.a { background: url(/bg.png), url( https://cdn.example.com/x.png ); }\n.b { background-image: image-set(\"https://cdn.example.com/y.png\" 1x); }\n";
    let expected = format!(
        "@import \"{cdn}/base.css\";\n.a {{ background: url(/bg.png), url({cdn}/x.png); }}\n.b {{ background-image: image-set(\"{cdn}/y.png\" 1x); }}\n"
    );
    assert_eq!(rewrite_css(source, BASE, false, &context), expected);
}

#[test]
fn a_selector_on_another_origins_address_matches_both_forms_and_reads_back_as_written() {
    let context = top_document("http://localhost:5173");
    let source = "a[href^=\"https://docs.example.com/\"] { color: red; }";
    let rewritten = rewrite_css(source, BASE, false, &context);
    let docs = preview_origin("https://docs.example.com", "http://localhost", true);
    assert!(rewritten.starts_with("a:is([href^=\"https://docs.example.com/\"]"), "{rewritten}");
    assert!(rewritten.contains(&format!("[href^=\"{docs}/\"]")), "{rewritten}");
    assert_eq!(rewrite_css(&rewritten, BASE, true, &context), source);
    // Paths do not change, so a selector on a path holds as written.
    let path = "a[href^=\"/docs\"] { color: red; }";
    assert_eq!(rewrite_css(path, BASE, false, &context), path);
}
