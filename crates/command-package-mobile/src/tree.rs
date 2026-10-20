//! The screen's tree as `inspect` shows it
//! (`crates/command-package-mobile/src/operations.rs`): each
//! element with a reference, its frame in screenshot pixels, and the
//! software keyboard as one Keyboard element with its frame unless its
//! keys are asked for.

use demi_command_package_mobile_protocol::mobile::Element;

use crate::{
    driver::{RawElement, Rect},
    refs::{Listed, Refs},
    scale::Scale,
};

/// The role and label of the collapsed keyboard.
pub const KEYBOARD_ROLE: &str = "keyboard";
pub const KEYBOARD_LABEL: &str = "Keyboard";

/// The tree's elements, referenced anew in `refs`.
pub fn elements(raw: &[RawElement], keyboard: bool, scale: &Scale, refs: &mut Refs) -> Vec<Element> {
    refs.renew();
    let raw = if keyboard { raw.to_vec() } else { collapse(raw) };
    raw.iter().map(|element| convert(element, scale, refs)).collect()
}

fn convert(raw: &RawElement, scale: &Scale, refs: &mut Refs) -> Element {
    let reference = refs.add(Listed {
        role: raw.role.clone(),
        label: raw.label.clone(),
        identifier: raw.identifier.clone(),
        frame: raw.frame,
    });
    Element {
        r#ref: reference,
        role: raw.role.clone(),
        label: raw.label.clone(),
        value: if raw.secure { None } else { raw.value.clone() },
        identifier: raw.identifier.clone(),
        frame: scale.frame(&raw.frame),
        enabled: raw.enabled,
        selected: raw.selected,
        focused: raw.focused,
        protected: raw.secure.then_some(true),
        children: raw
            .children
            .iter()
            .map(|child| convert(child, scale, refs))
            .collect(),
    }
}

/// `raw` with the keyboard's keys taken out and one Keyboard element, the
/// frame around them, after the window they were in.
fn collapse(raw: &[RawElement]) -> Vec<RawElement> {
    let mut frame: Option<Rect> = None;
    let kept = without_keys(raw, &mut frame);
    let Some(frame) = frame else {
        return kept;
    };
    let mut kept = kept;
    kept.push(RawElement {
        role: KEYBOARD_ROLE.into(),
        label: Some(KEYBOARD_LABEL.into()),
        frame,
        ..RawElement::default()
    });
    kept
}

fn without_keys(raw: &[RawElement], frame: &mut Option<Rect>) -> Vec<RawElement> {
    raw.iter()
        .filter_map(|element| {
            if element.keyboard_key {
                *frame = Some(match frame {
                    Some(around) => around.union(&element.frame),
                    None => element.frame,
                });
                return None;
            }
            let mut kept = element.clone();
            kept.children = without_keys(&element.children, frame);
            Some(kept)
        })
        .collect()
}

/// The elements in `raw`, depth first, that `keep` says.
pub fn find<'a>(raw: &'a [RawElement], keep: &dyn Fn(&RawElement) -> bool) -> Option<&'a RawElement> {
    for element in raw {
        if keep(element) {
            return Some(element);
        }
        if let Some(found) = find(&element.children, keep) {
            return Some(found);
        }
    }
    None
}
