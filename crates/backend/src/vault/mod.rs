//! Credential records, their scope and login flows (`providers.md` §
//! Credential vault), and the instance secret their keys derive from.

pub(crate) mod accounts;
pub(crate) mod entries;
pub(crate) mod logins;
pub(crate) mod operations;
pub(crate) mod pool;
pub(crate) mod quotas;
pub(crate) mod seal;
pub(crate) mod secret;
