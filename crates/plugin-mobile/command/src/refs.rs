//! A device's element references
//! (`crates/plugin-mobile/command/src/operations.rs`).
//!
//! Each `inspect` numbers the elements it lists on from the last number the
//! device gave, so a reference from an older inspect is never another
//! element's. An action resolves a reference to the centre of the frame the
//! inspect listed.

use std::collections::HashMap;

use demi_command_package_mobile_protocol::mobile::ElementRef;

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
}

impl Refs {
    /// Starts a new inspect's list: the elements before it are no longer
    /// referenced.
    pub fn renew(&mut self) {
        self.listed.clear();
    }

    /// The reference of the next listed element.
    pub fn add(&mut self, listed: Listed) -> ElementRef {
        self.last += 1;
        let reference = ElementRef::numbered(self.last);
        self.listed.insert(reference.clone(), listed);
        reference
    }

    pub fn get(&self, reference: &ElementRef) -> Option<&Listed> {
        self.listed.get(reference)
    }
}
