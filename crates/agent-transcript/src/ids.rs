//! Where the identities a session makes come from: block, turn and request
//! ids and a transcript's epoch. The source is injected so that a test can
//! make them predictable.

/// A source of identities, each one different from every other it gave.
pub trait IdSource {
    fn next_id(&self) -> String;
}

/// Random identities, a version 4 UUID's 32 hexadecimal digits each.
#[derive(Debug, Clone, Copy, Default)]
pub struct RandomIds;

impl IdSource for RandomIds {
    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }
}
