//! HTML to Markdown.
//!
//! 1. What a reader of the page never sees is removed first, since a page can
//!    hide instructions there.
//! 2. Each relative link and image source is made absolute against the page's
//!    URL, so the model can fetch it.
//! 3. htmd converts the rest: headings, lists, links, tables and code.
//!
//! The conversion leaves out what holds no text: an image's inline data, of
//! which only its alt text stays, and a link with nothing to read, such as an
//! icon's.

use std::rc::Rc;

use htmd::{
    Element, HtmlToMarkdown,
    element_handler::{HandlerResult, Handlers},
};
use markup5ever_rcdom::{Node, NodeData};
use reqwest::Url;

/// Elements whose content is no text of the page: scripts, styles, frames,
/// embedded objects and drawings.
const LEFT_OUT: [&str; 11] = [
    "script", "style", "template", "iframe", "frame", "frameset", "object", "embed", "svg",
    "canvas", "noembed",
];

/// The attributes that hold a URL, by the element that has them.
const URL_ATTRIBUTES: [(&str, &str); 2] = [("a", "href"), ("img", "src")];

/// The Markdown of `html`, the page at `url`.
pub fn markdown(html: &str, url: &Url) -> String {
    let converter = HtmlToMarkdown::builder()
        .skip_tags(LEFT_OUT.to_vec())
        // The page as a reader without JavaScript sees it: a noscript's
        // content is the page's, as the fetch runs no script.
        .scripting_enabled(false)
        .add_handler(vec!["img"], image)
        .add_handler(vec!["a"], link)
        .build();
    // Parsing a string reads nothing that can fail; html5ever recovers
    // from any markup.
    let tree = converter
        .html_to_tree(html)
        .expect("parsing a string performs no IO");
    remove_hidden(&tree);
    // The page's relative URLs are against its `<base href>`, as a browser
    // takes them, else against the page's own URL.
    let base = base_href(&tree)
        .and_then(|href| url.join(&href).ok())
        .unwrap_or_else(|| url.clone());
    make_absolute(&tree, &base);
    converter.tree_to_markdown(&tree)
}

/// The `href` of the first `<base>` at or below `node`.
fn base_href(node: &Rc<Node>) -> Option<String> {
    if let NodeData::Element { name, attrs, .. } = &node.data
        && &*name.local == "base"
        && let Some(href) = attrs.borrow().iter().find(|attribute| &*attribute.name.local == "href")
    {
        return Some(href.value.to_string());
    }
    node.children.borrow().iter().find_map(base_href)
}

/// Makes every link and image source below `node` absolute against `base`.
/// One that does not parse as a URL stays as the page wrote it.
fn make_absolute(node: &Rc<Node>, base: &Url) {
    if let NodeData::Element { name, attrs, .. } = &node.data {
        for (element, attribute) in URL_ATTRIBUTES {
            if &*name.local != element {
                continue;
            }
            for value in attrs.borrow_mut().iter_mut() {
                if &*value.name.local == attribute
                    && !value.value.trim_start().starts_with("data:")
                    && let Ok(absolute) = base.join(value.value.trim())
                {
                    value.value = absolute.as_str().into();
                }
            }
        }
    }
    for child in node.children.borrow().iter() {
        make_absolute(child, base);
    }
}

/// An image as htmd writes it, but one whose source is inline data, often
/// thousands of characters of an icon's SVG, by its alternative text alone,
/// set apart from the words around it as the image was.
fn image(handlers: &dyn Handlers, element: Element) -> Option<HandlerResult> {
    let attribute = |name: &str| {
        element
            .attrs
            .iter()
            .find(|attribute| &*attribute.name.local == name)
            .map(|attribute| attribute.value.to_string())
    };
    if attribute("src").is_some_and(|source| source.trim_start().starts_with("data:")) {
        let alt = attribute("alt").unwrap_or_default();
        let alt = alt.trim();
        return Some(if alt.is_empty() { String::new() } else { format!(" {alt} ") }.into());
    }
    handlers.fallback(element)
}

/// A link as htmd writes it, or nothing for one with no text to read.
fn link(handlers: &dyn Handlers, element: Element) -> Option<HandlerResult> {
    if handlers.walk_children(element.node).content.trim().is_empty() {
        return None;
    }
    handlers.fallback(element)
}

/// Removes every element below `node` that is hidden from view: `hidden`,
/// `aria-hidden="true"`, or an inline `display: none` or `visibility:
/// hidden`.
fn remove_hidden(node: &Rc<Node>) {
    node.children.borrow_mut().retain(|child| !hidden(child));
    for child in node.children.borrow().iter() {
        remove_hidden(child);
    }
}

fn hidden(node: &Node) -> bool {
    let NodeData::Element { attrs, .. } = &node.data else {
        return false;
    };
    attrs.borrow().iter().any(|attribute| {
        let value = attribute.value.to_ascii_lowercase();
        match &*attribute.name.local {
            "hidden" => true,
            "aria-hidden" => value.trim() == "true",
            "style" => {
                let style: String = value.chars().filter(|c| !c.is_whitespace()).collect();
                style
                    .split(';')
                    .any(|declaration| {
                        let declaration = declaration.trim_end_matches("!important");
                        declaration == "display:none" || declaration == "visibility:hidden"
                    })
            }
            _ => false,
        }
    })
}
