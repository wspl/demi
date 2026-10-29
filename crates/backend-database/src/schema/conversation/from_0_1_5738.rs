//! From 0.1.5738's conversation schema: checkpoint state format 10, whose
//! agent account names the rotation Smart where format 9 named Balanced.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(
    "UPDATE nodes SET state = json_set(state, '$.account.rotation', 'smart')
       WHERE json_extract(state, '$.account.rotation') = 'balanced';",
);
