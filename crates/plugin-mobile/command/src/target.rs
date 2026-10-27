//! How a command names an element (`crates/plugin-mobile/command/src/operations.rs`):
//! a ref from the last `inspect`, which stands for where that inspect listed
//! the element; or `text=` or `id=`, which the command finds in the tree as
//! it runs, once the element is there.
//!
//! An element is there once it is the only match, its centre is on the
//! screen, it is not disabled, and it stays at its place while the picture
//! settles: a page that slides in, or a window that opens, takes no input
//! along its way, and the tree may say where its element ends up before
//! it is there.

use std::time::Duration;

use tokio::time::Instant;

use demi_command_package_mobile_protocol::mobile::{ElementRef, ElementSummary};

use crate::{
    devices::OpenDevice,
    driver::{RawElement, Rect},
    error::MobileError,
    refs::Listed,
    scale::Scale,
    text,
};

/// The least time between two reads of the tree, for a device that answers
/// at once.
const READ_GAP: Duration = Duration::from_millis(50);

/// An element as a command names it.
pub enum Target {
    Ref(ElementRef),
    Find(Selector),
}

/// An element by what it shows or its identifier.
pub enum Selector {
    /// Its label or value is the text, spaces at either end and runs of
    /// spaces aside; or, when no element's is, holds it, case aside.
    Text(String),
    /// Its identifier, exactly.
    Id(String),
}

impl Target {
    pub fn parse(target: &str) -> Result<Self, MobileError> {
        if let Some(text) = target.strip_prefix("text=") {
            return Ok(Self::Find(Selector::Text(text.to_owned())));
        }
        if let Some(id) = target.strip_prefix("id=") {
            return Ok(Self::Find(Selector::Id(id.to_owned())));
        }
        target.parse().map(Self::Ref).map_err(|_: String| {
            MobileError::Usage(format!(
                "{target:?} is not an element: name one by a ref such as e7, text=<text it shows> or id=<identifier>"
            ))
        })
    }
}

impl Selector {
    /// The elements of `tree` it matches, the keyboard's keys aside.
    pub fn matches<'a>(&self, tree: &'a [RawElement]) -> Vec<&'a RawElement> {
        match self {
            Self::Id(id) => all(tree, &|element| element.identifier.as_deref() == Some(id.as_str())),
            Self::Text(text) => {
                let wanted = spaced(text);
                let exact = all(tree, &|element| shown(element).any(|shown| spaced(shown) == wanted));
                if !exact.is_empty() {
                    return exact;
                }
                let wanted = wanted.to_lowercase();
                all(tree, &|element| shown(element).any(|shown| spaced(shown).to_lowercase().contains(&wanted)))
            }
        }
    }

    /// What a match does, as a sentence says it of one element or of
    /// several: `shows "Delete"`, `show "Delete"`.
    pub fn said(&self, one: bool) -> String {
        match (self, one) {
            (Self::Text(text), true) => format!("shows {}", text::quoted(text)),
            (Self::Text(text), false) => format!("show {}", text::quoted(text)),
            (Self::Id(id), true) => format!("has the id {}", text::quoted(id)),
            (Self::Id(id), false) => format!("have the id {}", text::quoted(id)),
        }
    }
}

/// What `element` shows: its label and its value.
fn shown(element: &RawElement) -> impl Iterator<Item = &String> {
    [&element.label, &element.value].into_iter().flatten()
}

/// `text` with its spaces at either end dropped and its runs of spaces one.
fn spaced(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The elements of `tree`, depth first, that `keep` says, the keyboard's
/// keys aside.
fn all<'a>(tree: &'a [RawElement], keep: &dyn Fn(&RawElement) -> bool) -> Vec<&'a RawElement> {
    fn walk<'a>(tree: &'a [RawElement], keep: &dyn Fn(&RawElement) -> bool, into: &mut Vec<&'a RawElement>) {
        for element in tree {
            if !element.keyboard_key && keep(element) {
                into.push(element);
            }
            walk(&element.children, keep, into);
        }
    }
    let mut into = Vec::new();
    walk(tree, keep, &mut into);
    into
}

/// `element` with a ref of its own on `open`.
pub fn referenced(open: &OpenDevice, element: &RawElement) -> (ElementRef, Listed) {
    let listed = Listed {
        role: element.role.clone(),
        label: element.label.clone(),
        identifier: element.identifier.clone(),
        frame: element.frame,
    };
    let display = open.device.screen().borrow().fold.map(crate::driver::Folded::display);
    let mut refs = open.refs.lock().expect("the refs are intact");
    refs.on(display);
    let reference = refs.add(listed.clone());
    (reference, listed)
}

/// The element `selector` names on `open`, once it is there, waiting at
/// most `timeout`; with a ref of its own.
pub async fn find(open: &OpenDevice, selector: &Selector, timeout: Duration) -> Result<(ElementRef, Listed), MobileError> {
    /// Why the element is not there yet, which a timeout says.
    enum Short {
        Missing,
        OffScreen,
        Disabled,
        /// Two reads found it at different places.
        Moving,
        /// The picture did not settle around it.
        Unsettled,
    }
    let started = Instant::now();
    let deadline = started + timeout;
    let mut last: Option<Rect> = None;
    let mut short = Short::Missing;
    loop {
        let read = Instant::now();
        // A read under way at the deadline ends the wait, as `wait`'s does.
        let Ok(tree) = tokio::time::timeout_at(deadline, open.device.tree()).await else {
            break;
        };
        let tree = tree?;
        let matches = selector.matches(&tree);
        match matches.as_slice() {
            [] => {
                short = Short::Missing;
                last = None;
            }
            [element] => {
                let frame = element.frame;
                let screen = open.device.screen().borrow().points;
                let centre = frame.centre();
                let on_screen = (0.0..screen.x).contains(&centre.x) && (0.0..screen.y).contains(&centre.y);
                if !on_screen {
                    short = Short::OffScreen;
                } else if element.enabled == Some(false) {
                    short = Short::Disabled;
                } else if last.is_some_and(|last| near(&last, &frame)) {
                    return Ok(referenced(open, element));
                } else {
                    short = if last.is_some() { Short::Moving } else { Short::Unsettled };
                    last = Some(frame);
                    // The wait for a settled picture ends with the timeout too.
                    if tokio::time::timeout_at(deadline, crate::operations::settle(open)).await.is_err() {
                        break;
                    }
                    continue;
                }
                last = None;
            }
            several => return Err(MobileError::Failed(ambiguous(open, selector, several))),
        }
        if Instant::now() + READ_GAP >= deadline {
            break;
        }
        tokio::time::sleep_until(read + READ_GAP).await;
    }
    let seconds = timeout.as_secs();
    let what = match short {
        Short::Missing => format!("no element {}", selector.said(true)),
        Short::OffScreen => format!("the element that {} is off the screen; swipe to bring it on", selector.said(true)),
        Short::Disabled => format!("the element that {} is disabled", selector.said(true)),
        Short::Moving => format!("the element that {} is still moving", selector.said(true)),
        Short::Unsettled => format!(
            "the element that {} was found, but the screen around it was still changing",
            selector.said(true)
        ),
    };
    Err(MobileError::Timeout(format!("after {seconds} s {what}")))
}

/// Whether two frames of an element are the same place, a point aside.
fn near(a: &Rect, b: &Rect) -> bool {
    (a.x - b.x).abs() < 1.0 && (a.y - b.y).abs() < 1.0 && (a.width - b.width).abs() < 1.0 && (a.height - b.height).abs() < 1.0
}

/// The failure of a selector several elements match: each with a ref and
/// where it is, so the agent names the one it means.
fn ambiguous(open: &OpenDevice, selector: &Selector, several: &[&RawElement]) -> String {
    let scale = Scale::of(&open.device.screen().borrow());
    let listed: Vec<String> = several
        .iter()
        .map(|element| {
            let (reference, listed) = referenced(open, element);
            let point = scale.point(listed.frame.centre());
            let summary = ElementSummary {
                r#ref: reference,
                role: listed.role,
                label: listed.label,
            };
            format!("{} at {},{}", text::summary(&summary), point.x, point.y)
        })
        .collect();
    format!(
        "{} elements {}: {}; name the one you mean by its ref",
        several.len(),
        selector.said(false),
        listed.join(", ")
    )
}
