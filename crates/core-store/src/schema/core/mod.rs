//! The core database's history: each schema a core's database may hold
//! before the current one, oldest first, and the step from it to the next.
//! The first is the one a server's control database is split into
//! (`crates/backend-database/src/schema/control/`).

type Shipped = super::Shipped<()>;

pub(super) const HISTORY: &[Shipped] = &[];
