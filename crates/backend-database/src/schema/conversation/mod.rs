//! The conversation database's history: each schema a server may hold before the
//! current one, oldest first, and the step from it to the next.

use super::Shipped;

mod from_0_1_65_dev_contents_once;
mod from_0_1_5708;

pub(super) const HISTORY: &[Shipped] = &[
    Shipped {
        sql: include_str!("from_0_1_65_dev_contents_once.sql"),
        migration: from_0_1_65_dev_contents_once::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5708.sql"),
        migration: from_0_1_5708::MIGRATION,
    },
];
