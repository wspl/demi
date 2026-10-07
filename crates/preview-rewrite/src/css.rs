//! CSS rewriting: absolute resource addresses in `url()`, `@import` and `image-set()` become
//! preview addresses, and attribute selectors on URL attributes match the preview addresses the
//! DOM holds (`docs/browser/preview.md` § Addresses and labels); every other byte stays as it was. Works on whole
//! stylesheets, declaration lists (a `style` attribute), single values and selectors alike,
//! and leaves malformed CSS to the browser.

use cssparser::{ParseError, Parser, Token};

use crate::address::{Context, Role, convert_written_url, map_written_url};
use crate::attributes::URL_ATTRIBUTES;

/// One replacement of the source's bytes `start..end`.
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

struct Scan<'c> {
    base: &'c str,
    /// The base of the document whose elements selectors match: addresses in selectors are
    /// mapped as that document's attributes are.
    document_base: &'c str,
    reflect: bool,
    context: &'c Context,
    edits: Vec<Edit>,
}

#[derive(Clone, Copy, PartialEq)]
enum Block {
    Plain,
    /// Inside `url(` written with a quoted string.
    Url,
    /// Inside `image-set(`, whose strings are addresses.
    ImageSet,
}

fn quote(text: &str, quote: char) -> String {
    let mut output = String::with_capacity(text.len() + 2);
    output.push(quote);
    for character in text.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\a "),
            character if character == quote => {
                output.push('\\');
                output.push(character);
            }
            character => output.push(character),
        }
    }
    output.push(quote);
    output
}

/// An attribute selector the proxy changes: `[name op "value" flag]`.
struct AttributeSelector {
    name: String,
    operator: &'static str,
    value: String,
    flag: Option<String>,
}

fn attribute_selector(parser: &mut Parser<'_>) -> Result<AttributeSelector, ParseError<()>> {
    let name = parser.expect_ident()?.to_string();
    if !URL_ATTRIBUTES.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(ParseError::custom(()));
    }
    let operator = match parser.next()? {
        Token::Delim('=') => "=",
        Token::PrefixMatch => "^=",
        Token::SuffixMatch => "$=",
        Token::SubstringMatch => "*=",
        _ => return Err(ParseError::custom(())),
    };
    let value = match parser.next()? {
        Token::Ident(value) | Token::QuotedString(value) => value.to_string(),
        _ => return Err(ParseError::custom(())),
    };
    let flag = match parser.next() {
        Ok(Token::Ident(flag)) => Some(flag.to_string()),
        Ok(_) => return Err(ParseError::custom(())),
        Err(_) => None,
    };
    parser.expect_exhausted()?;
    Ok(AttributeSelector { name, operator, value, flag })
}

fn unquoted_url(text: &str) -> String {
    let mut output = String::from("url(");
    for character in text.chars() {
        if matches!(character, '\\' | '(' | ')' | '"' | '\'') || character.is_whitespace() {
            output.push_str(&format!("\\{:x} ", character as u32));
        } else {
            output.push(character);
        }
    }
    output.push(')');
    output
}

impl Scan<'_> {
    fn convert(&self, value: &str) -> String {
        convert_written_url(value, self.base, self.reflect, Role::Resource, self.context)
    }

    /// Where every preview address of this namespace starts.
    fn proxy_prefix(&self) -> String {
        self.context.namespace_prefix()
    }

    fn selector(&self, name: &str, operator: &str, value: &str, flag: Option<&str>) -> String {
        let flag = flag.map(|flag| format!(" {flag}")).unwrap_or_default();
        format!("[{name}{operator}{}{flag}]", quote(value, '"'))
    }

    /// The preview addresses an address written as `value` becomes in this document: as a
    /// resource, a module, a child frame's document and a navigation.
    fn written_forms(&self, value: &str) -> Vec<String> {
        let mut forms = Vec::new();
        for role in [Role::Resource, Role::Module, Role::Child, Role::Navigation, Role::TopNavigation] {
            let mapped = map_written_url(value, self.document_base, role, self.context);
            if mapped != value && !forms.contains(&mapped) {
                forms.push(mapped);
            }
        }
        forms
    }

    /// The prefixes of the preview addresses that start with `value`: only an absolute value
    /// is mapped in the DOM; a relative one, or part of a scheme, matches as written.
    fn proxy_prefixes(&self, value: &str) -> Vec<String> {
        if !value.contains("://") && !value.starts_with("//") {
            return Vec::new();
        }
        self.written_forms(value)
            .into_iter()
            // Parsing adds a slash after a bare host; the page's prefix had none.
            .map(|form| if value.ends_with('/') || form.contains("#to=") { form } else { form.strip_suffix('/').map_or(form.clone(), ToOwned::to_owned) })
            .collect()
    }

    /// The preview origins an absolute value's origin maps to, and the rest of the value.
    fn origin_and_rest(&self, value: &str) -> Option<(Vec<String>, String)> {
        let scheme_end = value.find("://")? + 3;
        let path_start = value[scheme_end..].find('/').map_or(value.len(), |index| scheme_end + index);
        let origins: Vec<String> = self
            .written_forms(&format!("{}/", &value[..path_start]))
            .into_iter()
            .filter_map(|form| url::Url::parse(&form).ok().map(|url| url.origin().ascii_serialization()))
            .fold(Vec::new(), |mut origins, origin| {
                if !origins.contains(&origin) {
                    origins.push(origin);
                }
                origins
            });
        (!origins.is_empty()).then(|| (origins, value[path_start..].to_owned()))
    }

    /// `:is(the page's selector, the preview addresses it means)`, or nothing when the DOM
    /// holds the value as written.
    fn attribute_alternatives(&self, attribute: &AttributeSelector, original: &str) -> Option<String> {
        let AttributeSelector { name, operator, value, flag } = attribute;
        let flag = flag.as_deref();
        let alternatives: Vec<String> = match *operator {
            "=" => self.written_forms(value).iter().map(|form| self.selector(name, "=", form, flag)).collect(),
            "^=" => self.proxy_prefixes(value).iter().map(|prefix| self.selector(name, "^=", prefix, flag)).collect(),
            _ => {
                let (origins, rest) = self.origin_and_rest(value)?;
                origins
                    .iter()
                    .map(|origin| if rest.is_empty() { self.selector(name, "^=", origin, flag) } else { format!("{}{}", self.selector(name, "^=", origin, flag), self.selector(name, operator, &rest, flag)) })
                    .collect()
            }
        };
        if alternatives.is_empty() {
            return None;
        }
        // The page's own selector keeps matching what it wrote, but not a preview address.
        let own = if *operator == "=" { original.to_owned() } else { format!("{original}:not({})", self.selector(name, "^=", &self.proxy_prefix(), None)) };
        Some(format!(":is({own},{})", alternatives.join(",")))
    }

    /// The page's selector back from `:is(…)` this rewriter made, given the text inside it.
    fn reflected_alternatives(&self, inner: &str) -> Option<String> {
        if !inner.starts_with('[') || !inner.contains(&format!("\"{}", self.proxy_prefix())) {
            return None;
        }
        let mut parser = Parser::new(inner);
        let Ok(Token::SquareBracketBlock) = parser.next_including_whitespace_and_comments() else { return None };
        let result: Result<(), ParseError<()>> = parser.parse_nested_block(|block| {
            while block.next_including_whitespace_and_comments().is_ok() {}
            Ok(())
        });
        result.ok()?;
        Some(inner[..parser.position().byte_index()].to_owned())
    }

    fn walk(&mut self, parser: &mut Parser<'_>, source: &str, offset: usize, block: Block) {
        let mut after_import = false;
        loop {
            let start = parser.position().byte_index();
            let token = match parser.next_including_whitespace_and_comments() {
                Ok(token) => token.clone(),
                Err(_) => break,
            };
            let end = parser.position().byte_index();
            match token {
                Token::UnquotedUrl(value) => {
                    let converted = self.convert(&value);
                    if converted != *value {
                        self.edits.push(Edit { start: offset + start, end: offset + end, text: unquoted_url(&converted) });
                    }
                    after_import = false;
                }
                Token::QuotedString(value) if after_import || block != Block::Plain => {
                    let converted = self.convert(&value);
                    if converted != *value {
                        let delimiter = source[offset + start..].chars().next().unwrap_or('"');
                        self.edits.push(Edit { start: offset + start, end: offset + end, text: quote(&converted, delimiter) });
                    }
                    after_import = false;
                }
                Token::Function(name) if self.reflect && name.eq_ignore_ascii_case("is") && source.as_bytes().get((offset + start).wrapping_sub(1)) == Some(&b':') => {
                    let inner_start = offset + start + name.len() + 1;
                    self.nested(parser, source, offset, Block::Plain);
                    let end = offset + parser.position().byte_index();
                    if let Some(original) = source.get(inner_start..end - 1).and_then(|inner| self.reflected_alternatives(inner)) {
                        self.edits.retain(|edit| edit.start < offset + start - 1 || edit.end > end);
                        self.edits.push(Edit { start: offset + start - 1, end, text: original });
                    }
                    after_import = false;
                }
                Token::SquareBracketBlock if !self.reflect => {
                    let state = parser.state();
                    let attribute = parser.parse_nested_block(attribute_selector);
                    let end = parser.position().byte_index();
                    match attribute {
                        Ok(attribute) => {
                            if let Some(text) = self.attribute_alternatives(&attribute, &source[offset + start..offset + end]) {
                                self.edits.push(Edit { start: offset + start, end: offset + end, text });
                            }
                        }
                        Err(_) => {
                            parser.reset(&state);
                            self.nested(parser, source, offset, Block::Plain);
                        }
                    }
                    after_import = false;
                }
                Token::Function(name) => {
                    let lower = name.to_ascii_lowercase();
                    let inner = match lower.as_str() {
                        "url" | "src" => Block::Url,
                        "image-set" | "-webkit-image-set" => Block::ImageSet,
                        _ => Block::Plain,
                    };
                    self.nested(parser, source, offset, inner);
                    after_import = false;
                }
                Token::CurlyBracketBlock | Token::SquareBracketBlock | Token::ParenthesisBlock => {
                    self.nested(parser, source, offset, Block::Plain);
                    after_import = false;
                }
                Token::AtKeyword(name) => after_import = name.eq_ignore_ascii_case("import"),
                Token::WhiteSpace(_) | Token::Comment(_) => {}
                _ => after_import = false,
            }
        }
    }

    fn nested(&mut self, parser: &mut Parser<'_>, source: &str, offset: usize, block: Block) {
        let result: Result<(), ParseError<()>> = parser.parse_nested_block(|inner| {
            self.walk(inner, source, offset, block);
            Ok(())
        });
        // A block that does not close is still the browser's to report; the addresses
        // found before the end are already recorded.
        drop(result);
    }
}

/// Rewrite CSS addresses against `base`; with `reflect`, convert proxy addresses back to
/// the text the page wrote. Selectors match the document whose base is `base`.
pub fn rewrite_css(source: &str, base: &str, reflect: bool, context: &Context) -> String {
    rewrite_stylesheet(source, base, base, reflect, context)
}

/// Rewrite a stylesheet whose addresses resolve against `base` and whose selectors match the
/// document at `document_base`, such as an external stylesheet.
pub fn rewrite_stylesheet(source: &str, base: &str, document_base: &str, reflect: bool, context: &Context) -> String {
    let mut parser = Parser::new(source);
    let mut scan = Scan { base, document_base, reflect, context, edits: Vec::new() };
    scan.walk(&mut parser, source, 0, Block::Plain);
    let mut output = source.to_owned();
    for edit in scan.edits.iter().rev() {
        output.replace_range(edit.start..edit.end, &edit.text);
    }
    output
}
