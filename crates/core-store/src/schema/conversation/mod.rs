//! The conversation database's history: each schema a server may hold before the
//! current one, oldest first, and the step from it to the next.

use super::ReadBlob;

type Shipped = super::Shipped<dyn ReadBlob>;
/// The step of each schema of the history, as its own module says it.
type Migration = super::Migration<dyn ReadBlob>;

mod from_0_1_65_dev_contents_once;
mod from_0_1_5708;
mod from_0_1_5738;
mod from_0_1_5742;
mod from_0_1_5785;
mod from_0_1_65_dev_directories_follow_moves;

pub(super) const HISTORY: &[Shipped] = &[
    Shipped {
        sql: include_str!("from_0_1_65_dev_contents_once.sql"),
        migration: from_0_1_65_dev_contents_once::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5708.sql"),
        migration: from_0_1_5708::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5738.sql"),
        migration: from_0_1_5738::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_5742.sql"),
        migration: from_0_1_5742::MIGRATION,
    },    Shipped {
        sql: include_str!("from_0_1_5785.sql"),
        migration: from_0_1_5785::MIGRATION,
    },
    Shipped {
        sql: include_str!("from_0_1_65_dev_directories_follow_moves.sql"),
        migration: from_0_1_65_dev_directories_follow_moves::MIGRATION,
    },
];
