//! HTML to Markdown (`web.md` § demi web fetch): what a reader of the page
//! never sees is removed first, since a page can hide instructions there,
//! then htmd converts the rest, headings, lists, links, tables and code
//! included, leaving out what holds no text: an image's inline data and a
//! link with nothing to read, such as an icon's.

use std::rc::Rc;

use htmd::{
    Element, HtmlToMarkdown,
    element_handler::{HandlerResult, Handlers},
};
use markup5ever_rcdom::{Node, NodeData};

/// Elements whose content is no text of the page: scripts, styles, frames,
/// embedded objects and drawings.
const LEFT_OUT: [&str; 11] = [
    "script", "style", "template", "iframe", "frame", "frameset", "object", "embed", "svg",
    "canvas", "noembed",
];

/// The Markdown of `html`.
pub fn markdown(html: &str) -> String {
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
    converter.tree_to_markdown(&tree)
}

/// An image as htmd writes it, but one whose source is inline data, often
/// thousands of characters of an icon's SVG, by its alternative text alone.
fn image(handlers: &dyn Handlers, element: Element) -> Option<HandlerResult> {
    let attribute = |name: &str| {
        element
            .attrs
            .iter()
            .find(|attribute| &*attribute.name.local == name)
            .map(|attribute| attribute.value.to_string())
    };
    if attribute("src").is_some_and(|source| source.trim_start().starts_with("data:")) {
        return Some(attribute("alt").unwrap_or_default().into());
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
