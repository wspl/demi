//! The Host's number a read was taken at (`runner.md` § Watching files):
//! the count of file system events the Host's runner had received when the
//! read began, and the epoch that names the runner's run.

use std::fmt;

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::MAX_SAFE_INTEGER;

/// Where in the Host's run of numbers a read began. An answer read at 40 is
/// out of date once a change at 41 is reported; one read at 41 already
/// holds it. Numbers of another epoch, another run of the runner, compare
/// with nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct ReadAt {
    #[garde(ascii, length(min = 1, max = 64))]
    pub epoch: String,
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub number: u64,
}

impl fmt::Display for ReadAt {
    /// `<epoch>.<number>`, as `X-Demi-Read-At` carries it (`web-api-hosts.md`
    /// § File text and working tree changes).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.epoch, self.number)
    }
}

impl std::str::FromStr for ReadAt {
    type Err = String;

    /// `<epoch>.<number>`, as [`ReadAt`]'s `Display` writes it; the epoch
    /// may hold dots of its own, the number never does.
    fn from_str(text: &str) -> Result<Self, String> {
        let refuse = || format!("must be <epoch>.<number>, not {text:?}");
        let (epoch, number) = text.rsplit_once('.').ok_or_else(refuse)?;
        let read_at = ReadAt {
            epoch: epoch.to_owned(),
            number: number.parse().map_err(|_| refuse())?,
        };
        read_at.validate().map_err(|error| format!("{text:?}: {error}"))?;
        Ok(read_at)
    }
}

impl ReadAt {
    /// Whether a read taken here holds what was there at `after`: of the
    /// same run, at its number or later.
    pub fn holds(&self, after: &ReadAt) -> bool {
        self.epoch == after.epoch && self.number >= after.number
    }

    /// The earlier of two reads of one answer, which holds what both hold
    /// only from there: a change after it may be missing from either.
    pub fn earlier(self, other: ReadAt) -> ReadAt {
        if self.epoch == other.epoch && other.number < self.number {
            other
        } else {
            self
        }
    }
}
