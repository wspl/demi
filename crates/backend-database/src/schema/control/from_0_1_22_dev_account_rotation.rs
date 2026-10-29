//! From the control schema with account rotation, whose placeholder titles
//! held the English "New conversation", to the one where a placeholder
//! title holds no words.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_ACCOUNT_ROTATION);

const CONTROL_FROM_ACCOUNT_ROTATION: &str =
    "UPDATE conversations SET title = '' WHERE title_origin = 'placeholder';";
