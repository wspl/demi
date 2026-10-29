//! What more than one step of the history uses.

/// Fork metadata format 2, without the destination's workflow, as the
/// control schema before [`control_from_0_1_22_dev_20261015t105826z`]
/// stored it.
pub(super) mod fork_format_2 {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(in super::super) struct ForkMetadata {
        #[serde(deserialize_with = "Option::deserialize")]
        pub title: Option<String>,
        /// The target as the release that stored it wrote it, which a later
        /// migration rewrites (`control_from_0_1_5614`).
        pub target: serde_json::Value,
        #[serde(deserialize_with = "Option::deserialize")]
        pub model: Option<serde_json::Value>,
        pub created_at: serde_json::Value,
        pub attached_hosts: Vec<AttachedHost>,
        pub context_version: u64,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(in super::super) struct AttachedHost {
        pub device: String,
        #[serde(deserialize_with = "Option::deserialize")]
        pub cwd: Option<String>,
    }
}

/// Gives the time trigger `value` the field name `timing` in place of `schedule`.
pub(super) fn named_timing(value: &mut serde_json::Value) {
    if value.get("kind").and_then(serde_json::Value::as_str) == Some("time")
        && let Some(fields) = value.as_object_mut()
        && let Some(timing) = fields.remove("schedule")
    {
        fields.insert("timing".to_owned(), timing);
    }
}
