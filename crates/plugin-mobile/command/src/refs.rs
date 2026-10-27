//! A device's element references
//! (`crates/plugin-mobile/command/src/operations.rs`).
//!
//! Each `inspect` numbers the elements it lists on from the last number the
//! device gave, so a reference from an older inspect is never another
//! element's. An action resolves a reference to the centre of the frame the
//! inspect listed. A foldable that lights its other display leaves the
//! references listed on the one before stale, whoever folded it.

use std::collections::HashMap;

use demi_command_package_mobile_protocol::mobile::{ElementRef, FoldDisplay};

use crate::driver::{RawElement, Rect};

/// What a reference names: the element as its inspect listed it.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub role: String,
    pub label: Option<String>,
    pub identifier: Option<String>,
    /// In points of the upright screen.
    pub frame: Rect,
}

impl Listed {
    /// Whether `element` is the same element, as a later read finds it: its
    /// role, label and identifier.
    pub fn matches(&self, element: &RawElement) -> bool {
        self.role == element.role
            && self.label == element.label
            && self.identifier == element.identifier
    }
}

#[derive(Debug, Default)]
pub struct Refs {
    /// The last number given.
    last: u64,
    /// The last inspect's elements.
    listed: HashMap<ElementRef, Listed>,
    /// The foldable's display they were listed on.
    display: Option<FoldDisplay>,
}

impl Refs {
    /// Starts a new inspect's list: the elements before it are no longer
    /// referenced.
    pub fn renew(&mut self) {
        self.listed.clear();
    }

    /// The elements are listed on `display` from now on: those listed on
    /// another are no longer referenced.
    pub fn on(&mut self, display: Option<FoldDisplay>) {
        if self.display != display {
            self.listed.clear();
            self.display = display;
        }
    }

    /// The reference of the next listed element.
    pub fn add(&mut self, listed: Listed) -> ElementRef {
        self.last += 1;
        let reference = ElementRef::numbered(self.last);
        self.listed.insert(reference.clone(), listed);
        reference
    }

    /// The element `reference` names, while `display` is the one it was
    /// listed on.
    pub fn get(&self, reference: &ElementRef, display: Option<FoldDisplay>) -> Option<&Listed> {
        if self.display != display {
            return None;
        }
        self.listed.get(reference)
    }
}
