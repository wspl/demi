//! Query parameters (`web-api.md` § Query parameters). Queries are read by
//! the backend alone and are not part of the generated TypeScript.

use serde::{Deserialize, Deserializer, de};

/// A boolean query parameter: spelled exactly `true` or `false`, and false
/// when omitted. Anything else, such as `1`, `TRUE` or an empty value, is
/// refused rather than read as one of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StrictBool(pub bool);

impl<'de> Deserialize<'de> for StrictBool {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
        match text.as_ref() {
            "true" => Ok(Self(true)),
            "false" => Ok(Self(false)),
            other => Err(de::Error::custom(format!(
                "must be true or false, not {other:?}"
            ))),
        }
    }
}

/// A Host read number spelled `<epoch>.<number>`, as `X-Demi-Read-At`
/// carries it, such as `?readAfter=` of the workflows routes; none when
/// omitted.
pub fn read_at<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<demi_shared_types::ReadAt>, D::Error> {
    let text = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
    text.parse().map(Some).map_err(de::Error::custom)
}

/// `?refresh=true|false`.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct Refresh {
    #[serde(default)]
    pub refresh: StrictBool,
}

/// `?q=<query>&entry=<id>` of `GET /models/search`: the words to match,
/// and the entry whose vendor fills the matches it lists.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelSearch {
    pub q: String,
    #[serde(default)]
    pub entry: Option<crate::ids::ProviderId>,
}
