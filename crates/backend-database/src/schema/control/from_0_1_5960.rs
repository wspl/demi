//! From 0.1.5960's control schema: using the user's other conversations asks
//! nothing. The grants of `conversation.use` go, and so do the requests for
//! it: an undecided one would ask for a category nothing declares, and a
//! decided one not yet delivered would tell the agent of one.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5960);

const CONTROL_FROM_0_1_5960: &str = "
DELETE FROM permission_grants WHERE category = 'conversation.use';
DELETE FROM permission_requests
  WHERE EXISTS (SELECT 1 FROM json_each(categories) WHERE value = 'conversation.use');
";
