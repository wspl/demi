//! The rules of `shared-types` at its boundary: the wire shape of a block and
//! what its decode refuses, its time and completion-id encodings, the lookups
//! the product shares with the web app, the table output, and the conversion
//! of a catalog model into a selection.

mod blocks;
mod encodings;
mod lookups;
mod output;
mod selection;
