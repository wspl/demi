//! Automations (`automations.md`): what the Automations page reads and
//! changes, and the rules each part follows wherever it arrives, a page's
//! body or an agent's `demi automation` alike. The backend's scheduler reads
//! the schedule; this crate only carries it.

use demi_shared_types::{AutomationId, AutomationRunId, Nullable, Timestamp};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::{double_option, unwrap_or_skip};

use crate::conversations::{ConversationTarget, ModelSettings};
use crate::ids::ConversationId;
use crate::text::Trimmed;

/// The most characters of an automation's name, after trimming.
pub const NAME_MAX: usize = 120;
/// The most bytes of a prompt.
pub const PROMPT_MAX_BYTES: usize = 64 * 1024;
/// The most bytes of a change watch's command.
pub const COMMAND_MAX_BYTES: usize = 16 * 1024;
/// The longest interval, a year, in minutes.
pub const INTERVAL_MAX_MINUTES: u32 = 366 * 24 * 60;
/// The most runs a day a limit allows.
pub const RUNS_PER_DAY_MAX: u32 = 10_000;
/// The most bytes of a Stripe signing secret the user pastes.
pub const STRIPE_SECRET_MAX_BYTES: usize = 256;
/// The runs an automation lists, newest first; older ones are kept until
/// the automation is deleted, and are not shown.
pub const RUNS_SHOWN: usize = 50;
/// The name a new automation starts with.
pub const NEW_NAME: &str = "New Automation";

/// A day of the week, as a daily schedule names the days it runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub const ALL: [Weekday; 7] = [
        Self::Mon,
        Self::Tue,
        Self::Wed,
        Self::Thu,
        Self::Fri,
        Self::Sat,
        Self::Sun,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mon => "mon",
            Self::Tue => "tue",
            Self::Wed => "wed",
            Self::Thu => "thu",
            Self::Fri => "fri",
            Self::Sat => "sat",
            Self::Sun => "sun",
        }
    }

    /// The day `text` names, `mon` to `sun`.
    pub fn named(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|day| day.as_str() == text)
    }
}

/// When a time trigger fires, in the automation's time zone
/// (`automations.md` § An automation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Schedule {
    /// Once, at this moment.
    Once {
        #[garde(skip)]
        at: Timestamp,
    },
    /// Every day of `days` at `time`, a clock time as `09:00`.
    Daily {
        #[garde(pattern(r"^([01][0-9]|2[0-3]):[0-5][0-9]$"))]
        time: String,
        /// One to seven distinct days.
        #[garde(length(min = 1, max = 7), custom(distinct_days))]
        days: Vec<Weekday>,
    },
    /// Every `minutes`, from when the schedule was set.
    Every {
        #[garde(range(min = 1, max = INTERVAL_MAX_MINUTES))]
        minutes: u32,
    },
    /// A cron expression of five fields: minute, hour, day of month, month
    /// and day of the week, which the backend reads.
    Cron {
        #[garde(length(chars, min = 9, max = 200))]
        expression: String,
    },
}

fn distinct_days(days: &[Weekday], _: &()) -> garde::Result {
    let mut seen = std::collections::BTreeSet::new();
    if days.iter().all(|day| seen.insert(*day)) {
        Ok(())
    } else {
        Err(garde::Error::new("a day is named twice"))
    }
}

/// What starts an automation's runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Trigger {
    /// At a time.
    Time {
        #[garde(dive)]
        schedule: Schedule,
    },
    /// When the output of `command`, run every `everyMinutes` on the
    /// target's device in the target's directory, changes.
    Change {
        #[garde(length(min = 1, max = COMMAND_MAX_BYTES), custom(not_blank))]
        command: String,
        #[garde(range(min = 1, max = INTERVAL_MAX_MINUTES))]
        every_minutes: u32,
    },
    /// When its URL, `<server>/hooks/<id>`, is called with its secret.
    Url {},
}

impl Trigger {
    /// Whether it takes a limit on runs a day: a change watch and a URL do.
    pub fn takes_limit(&self) -> bool {
        !matches!(self, Self::Time { .. })
    }
}

fn not_blank(text: &str, _: &()) -> garde::Result {
    if demi_shared_types::is_blank(text) {
        Err(garde::Error::new("is blank"))
    } else {
        Ok(())
    }
}

/// Where an automation's runs work (`automations.md` § An automation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AutomationTarget {
    /// A new conversation each run, there: a device, and a project on it or
    /// none, as a new conversation's target names them.
    New {
        #[garde(dive)]
        target: ConversationTarget,
    },
    /// An existing conversation, which each run wakes with a message.
    Existing {
        #[garde(skip)]
        conversation_id: ConversationId,
    },
}

/// Why an automation paused itself; none when the user paused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    /// Its existing conversation was archived.
    ConversationArchived,
    /// Its existing conversation was deleted.
    ConversationDeleted,
}

/// Where a run stands (`automations.md` § Runs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Done,
    /// Its agent ran `demi notify`.
    Notified,
    Skipped,
    Failed,
}

/// Why a run was skipped or failed, which the page words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RunNote {
    /// Skipped: the target's device was offline.
    Offline { device: String },
    /// Skipped: the day's limit of runs was reached; `calls` URL calls, or
    /// changes, came since, and started nothing.
    Limit { limit: u32, calls: u32 },
    /// Skipped: its existing conversation was archived or deleted, which
    /// paused the automation.
    TargetGone,
    /// Failed: the watch's command ran over 60 seconds and was stopped.
    CommandTimedOut,
    /// Failed: the watch's command could not run, for this reason.
    CommandFailed { message: String },
    /// Skipped: a new conversation needs the automation's model, and it has
    /// none chosen.
    NoModel,
    /// Failed: the run could not start its turn, or its turn failed, with
    /// this message and, from the turn's error block, its code, which the
    /// page words as the transcript words it.
    TurnFailed {
        message: String,
        #[serde(deserialize_with = "Option::deserialize")]
        #[schemars(with = "Nullable<String>")]
        code: Option<String>,
    },
}

/// One run of an automation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRunDto {
    pub id: AutomationRunId,
    /// When its trigger fired.
    pub at: Timestamp,
    pub state: RunState,
    /// Why it was skipped or failed.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<RunNote>")]
    pub note: Option<RunNote>,
    /// The start of its agent's last answer.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub line: Option<String>,
    /// The conversation it ran in; null for a run that started none, such
    /// as a skipped one, or whose conversation was deleted.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ConversationId>")]
    pub conversation_id: Option<ConversationId>,
    /// It notified, and the user has not opened its conversation since.
    pub unread: bool,
    /// Its new conversation is still filed under the automation, out of the
    /// sidebar; false once the user answered it or moved it, and for a run
    /// in an existing conversation.
    pub filed: bool,
}

/// An automation as the page lists and edits it, with its latest runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDto {
    pub id: AutomationId,
    pub name: String,
    pub trigger: Trigger,
    /// The IANA time zone its schedule and its daily limit count in.
    pub time_zone: String,
    /// A change watch's or a URL's limit on runs a day; null for none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<u32>")]
    pub max_runs_per_day: Option<u32>,
    pub target: AutomationTarget,
    pub prompt: String,
    /// The model settings a new conversation starts with; null for none
    /// chosen, which skips a run that needs a new conversation.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<ModelSettings>")]
    pub model: Option<ModelSettings>,
    pub paused: bool,
    /// Why it paused itself; null while it is on or the user paused it.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<PauseReason>")]
    pub paused_because: Option<PauseReason>,
    /// Whether a URL trigger checks Stripe's signatures, with the signing
    /// secret the user pasted.
    pub stripe_signing: bool,
    /// When it next fires by itself; null while paused or with no time
    /// left, and for a URL.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    pub next_at: Option<Timestamp>,
    pub created_at: Timestamp,
    /// Its latest runs, newest first, [`RUNS_SHOWN`] at most.
    pub runs: Vec<AutomationRunDto>,
}

/// `POST /automations` and `PATCH /automations/:id`: what changes; a new
/// automation starts from its defaults, *New Automation*, paused, daily at
/// 09:00 in a new conversation on the Cloud, and takes these over them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationPatch {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed")]
    #[garde(inner(length(chars, min = 1, max = NAME_MAX)))]
    pub name: Option<Trimmed>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "Trigger")]
    #[garde(dive)]
    pub trigger: Option<Trigger>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(inner(length(min = 1, max = 64)))]
    pub time_zone: Option<String>,
    /// Null removes the limit.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Nullable<u32>")]
    #[garde(inner(inner(range(min = 1, max = RUNS_PER_DAY_MAX))))]
    pub max_runs_per_day: Option<Option<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "AutomationTarget")]
    #[garde(dive)]
    pub target: Option<AutomationTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    #[garde(inner(length(max = PROMPT_MAX_BYTES)))]
    pub prompt: Option<String>,
    /// Null for the user's own model for new conversations.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Nullable<ModelSettings>")]
    #[garde(dive)]
    pub model: Option<Option<ModelSettings>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "bool")]
    #[garde(skip)]
    pub paused: Option<bool>,
    /// The signing secret Stripe shows for the endpoint, `whsec_…`; null
    /// stops checking Stripe's signatures.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
    #[schemars(with = "Nullable<String>")]
    #[garde(inner(inner(length(min = 1, max = STRIPE_SECRET_MAX_BYTES))))]
    pub stripe_secret: Option<Option<String>>,
}

/// The answer of a create, a change, a Run Now and a Move to
/// Conversations: the automation as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AutomationAnswer {
    pub automation: AutomationDto,
}

/// `GET /automations`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AutomationList {
    pub automations: Vec<AutomationDto>,
}

/// `GET` and `POST /automations/:id/secret`: a URL trigger's secret, which
/// the page shows behind Show; a `POST` makes a new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AutomationSecret {
    pub secret: String,
}

/// The run of an automation a conversation is filed under, out of the
/// sidebar, until the user answers it or moves it (`automations.md` § Runs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FiledRun {
    pub automation_id: AutomationId,
    pub run_id: AutomationRunId,
    /// When its trigger fired, which the run's bar shows whatever the
    /// run's age.
    pub at: Timestamp,
}
