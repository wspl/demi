//! The caller's personal instructions (`web-api.md` § Instructions), which
//! every node of the caller's conversations follows. The length rule is the
//! accounts service's, which checks a body before it is written.

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `PUT /instructions { text }`: the text replaces the personal
/// instructions; an empty or blank one removes them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct InstructionsBody {
    #[garde(skip)]
    pub text: String,
}
