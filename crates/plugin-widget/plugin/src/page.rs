//! A widget's page as the reply draws it: its own document, with the
//! bootstrap in front of its head.
//!
//! The bootstrap holds, in order:
//!
//! - the page's encoding, UTF-8, which the page's text is;
//! - a `<base>` that opens every link in a new tab;
//! - Tailwind's browser build from its CDN, pinned to the product's major
//!   release;
//! - a `@theme` with the color and font names of the product's own Tailwind
//!   theme (`packages/web-ui/src/styles/base.css`), so `bg-surface` is the
//!   product's surface, and the same names as the root's properties, which a
//!   script reads before Tailwind's build has generated its own;
//! - the product's theme values, its stylesheet's light and dark blocks, and
//!   its appearance stylesheet, which follow the root's `data-theme`,
//!   `data-tone` and `data-accent`, as the product's own pages do;
//! - the base sheet;
//! - the script that sets the root's appearance from the frame's name, or
//!   the default look in a page named nothing, and reports the page's
//!   height.
//!
//! A frame whose appearance changes draws its page again under its new name:
//! a chart a script drew holds the colors it read.
//!
//! The page's own head comes after, so its `@theme` and CSS override any of
//! it name by name.

use crate::markup;

/// The product's Tailwind theme, whose color and font names a widget takes.
const PRODUCT_THEME: &str = include_str!("../../../../packages/web-ui/src/styles/base.css");

/// The product's appearance, whose values a widget takes.
const APPEARANCE: &str = include_str!("../../../../packages/web-ui/src/styles/product-appearance.css");

const BASE: &str = include_str!("base.css");
const SCRIPT: &str = include_str!("bootstrap.js");

/// Tailwind's browser build: the product's major release, so the classes a
/// widget writes are the ones the product's own pages use.
const TAILWIND: &str = "https://cdn.jsdelivr.net/npm/@tailwindcss/browser@4";

/// The appearance the product fixes for every user
/// (`packages/web-ui/src/theme/productAppearance.ts`), and the tone and
/// accent a user starts with, which a widget drawn away from a reply takes,
/// as in its screenshots.
pub(crate) const DEFAULT_LOOK: [(&str, &str); 5] = [
    ("data-tone", "ink"),
    ("data-accent", "blue"),
    ("data-density", "regular"),
    ("data-radius", "medium"),
    ("data-shadow", "hairline"),
];

/// `DEFAULT_LOOK` as the bootstrap takes it, an object of root attributes.
fn default_look() -> String {
    let look: serde_json::Map<String, serde_json::Value> = DEFAULT_LOOK
        .iter()
        .map(|(name, value)| ((*name).to_owned(), serde_json::Value::from(*value)))
        .collect();
    serde_json::Value::Object(look).to_string()
}

/// `page` with the bootstrap in front of its head, and `prelude`, markup
/// that runs before the bootstrap, in front of that.
pub(crate) fn bootstrapped(page: &str, prelude: &str) -> String {
    let bootstrap = format!(
        "<meta charset=\"utf-8\">{prelude}<base target=\"_blank\"><script src=\"{TAILWIND}\"></script>\
         <style type=\"text/tailwindcss\">@theme {{\n{names}}}</style>\
         <style>:root {{\n{names}}}</style>\
         <style>{values}{APPEARANCE}</style><style>{BASE}</style><script>({SCRIPT})({look});</script>",
        names = theme_names(),
        values = theme_values(),
        look = default_look(),
    );
    let scanned = markup::scan(page);
    if let Some((_, at)) = markup::find_tag(&scanned, "head") {
        return format!("{}{bootstrap}{}", &page[..at], &page[at..]);
    }
    if let Some((_, at)) = markup::find_tag(&scanned, "html") {
        return format!("{}<head>{bootstrap}</head>{}", &page[..at], &page[at..]);
    }
    // After a doctype, which a document that goes before it would lose.
    if let Some((start, at)) = markup::find_tag(&scanned, "!doctype")
        && scanned[..start].trim().is_empty()
    {
        return format!("{}<head>{bootstrap}</head>{}", &page[..at], &page[at..]);
    }
    format!("<head>{bootstrap}</head>{page}")
}

/// The product's light and dark theme blocks, `[data-theme="..."] { ... }`.
fn theme_values() -> String {
    ["[data-theme=\"dark\"] {", "[data-theme=\"light\"] {"]
        .iter()
        .map(|open| {
            let start = PRODUCT_THEME
                .find(&format!("\n{open}"))
                .expect("the product's stylesheet has its light and dark themes")
                + 1;
            let end = PRODUCT_THEME[start..]
                .find("\n}")
                .map_or(PRODUCT_THEME.len(), |at| start + at + 2);
            format!("{}\n", &PRODUCT_THEME[start..end])
        })
        .collect()
}

/// The color and font declarations of the product's `@theme`.
fn theme_names() -> String {
    let start = PRODUCT_THEME
        .find("@theme {")
        .expect("the product's stylesheet has its Tailwind theme");
    PRODUCT_THEME[start..]
        .lines()
        .skip(1)
        .take_while(|line| line.trim() != "}")
        .map(str::trim)
        .filter(|line| line.starts_with("--color-") || line.starts_with("--font-"))
        .fold(String::new(), |mut names, line| {
            names.push_str(line);
            names.push('\n');
            names
        })
}

/// The page's title as a browser shows it, its `<title>`'s text with its
/// references read and its spaces collapsed; none for a page without one or
/// with one of no text.
pub(crate) fn title(page: &str) -> Option<String> {
    let scanned = markup::scan(page);
    let (start, open) = markup::find_tag(&scanned, "title")?;
    // An SVG's `<title>` names its drawing, not the page.
    if markup::find_tag(&scanned, "svg").is_some_and(|(svg, _)| svg < start) {
        return None;
    }
    // The title's text is blanked in the scan, its end tag is not.
    let close = open + scanned[open..].find("</title")?;
    let title = decoded(&page[open..close]).split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// `text` with its character references, named and numeric, read as a
/// browser reads a title's; one that names nothing stays as written.
fn decoded(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        decoded.push_str(&rest[..at]);
        rest = &rest[at..];
        let reference = rest[1..].find(';').map(|end| &rest[1..1 + end]);
        let characters = reference.and_then(|name| match name.strip_prefix('#') {
            Some(number) => number
                .strip_prefix(['x', 'X'])
                .map_or_else(|| number.parse::<u32>(), |hex| u32::from_str_radix(hex, 16))
                .ok()
                // As a browser does, a reference to no character is U+FFFD.
                .map(|code| char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{fffd}').to_string()),
            None => entities::ENTITIES
                .iter()
                .find(|entity| entity.entity.strip_prefix('&').and_then(|entity| entity.strip_suffix(';')) == Some(name))
                .map(|entity| entity.characters.to_owned()),
        });
        match (reference, characters) {
            (Some(name), Some(characters)) => {
                decoded.push_str(&characters);
                rest = &rest[name.len() + 2..];
            }
            _ => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}
