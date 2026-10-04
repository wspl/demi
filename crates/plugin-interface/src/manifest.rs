//! What a plugin declares once, when the backend starts (`plugins.md`
//! § What a plugin contributes): fixed for the life of the process.

use std::fmt;

use demi_command_declarations::{NativeOperation, Node, Schema};
use demi_shared_types::Profile;
use schemars::{JsonSchema, generate::SchemaSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The context source the product's execution context answers as, which no
/// plugin may be named.
pub const EXECUTION_SOURCE: &str = "execution";

/// The root the plugins of this repository place their groups under.
pub const DEMI_ROOT: &str = "demi";

/// The `demi` root's summary in the model's command help.
pub const DEMI_SUMMARY: &str =
    "The Demi platform command: every subcommand is a platform domain (file, browser, …).";

/// A plugin's id, such as `browser`: 1 to 32 lowercase letters, digits and
/// hyphens. It names the plugin's context blocks, values, part of the
/// product state, directories on a Host and page route.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(String);

/// Why a text is not a plugin's id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "\"{0}\" is not a plugin id: 1 to 32 lowercase letters, digits and hyphens, not \"{EXECUTION_SOURCE}\""
)]
pub struct PluginIdError(String);

impl PluginId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PluginId {
    type Error = PluginIdError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        let valid = (1..=32).contains(&id.len())
            && id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && id != EXECUTION_SOURCE;
        if valid {
            Ok(Self(id))
        } else {
            Err(PluginIdError(id))
        }
    }
}

impl TryFrom<&str> for PluginId {
    type Error = PluginIdError;

    fn try_from(id: &str) -> Result<Self, Self::Error> {
        Self::try_from(id.to_owned())
    }
}

impl From<PluginId> for String {
    fn from(id: PluginId) -> Self {
        id.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A plugin's declarations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: PluginId,
    /// Its name in settings: a feature's name, so in title style, such as
    /// `Conversation Browser` (the gallery's Writing page has the rule).
    pub name: String,
    /// What it does, in one sentence in sentence style, which settings show.
    pub description: String,
    /// Its command groups and roots (`plugins.md` § Commands).
    #[serde(default)]
    pub commands: Vec<Commands>,
    /// Its subagent profiles (`plugins.md` § Profiles).
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Whether it is a context source, asked before each provider request
    /// of a node while its user has it on (`plugins.md` § Prompt text and
    /// context).
    #[serde(default)]
    pub context: bool,
    /// Its user streams (`plugins.md` § Calling its command package).
    #[serde(default)]
    pub streams: Vec<Stream>,
    /// Its part of the web app's data and calls (`plugins.md` § The page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<Page>,
}

impl Manifest {
    /// A manifest that declares nothing yet.
    pub fn new(id: PluginId, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: description.into(),
            commands: Vec::new(),
            profiles: Vec::new(),
            context: false,
            streams: Vec::new(),
            page: None,
        }
    }
}

/// A user stream: a page connects to `operation` on the conversation's primary
/// Host for as long as it keeps the stream open. Its messages and constants
/// are declared for the page's generated types (`plugin-pages.md` § Types);
/// the backend relays the stream's bytes without reading them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stream {
    /// The name the page opens it by, taken once among all plugins.
    pub name: String,
    pub operation: NativeOperation,
    /// The messages the page receives.
    pub receives: Schema,
    /// The messages the page sends.
    pub sends: Schema,
    /// The constants the stream's two ends share, such as its frame kinds.
    #[serde(default)]
    pub constants: Vec<Constant>,
}

impl Stream {
    /// The stream `name` to `operation`, whose page receives `R` and sends
    /// `S`.
    pub fn new<R: JsonSchema, S: JsonSchema>(
        name: impl Into<String>,
        operation: NativeOperation,
    ) -> Self {
        Self {
            name: name.into(),
            operation,
            receives: schema_of::<R>(Direction::Serializes),
            sends: schema_of::<S>(Direction::Deserializes),
            constants: Vec::new(),
        }
    }

    pub fn constant(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        value: impl Into<Value>,
    ) -> Self {
        self.constants.push(Constant {
            name: name.into(),
            description: description.into(),
            value: value.into(),
        });
        self
    }
}

/// A value both ends of a stream use, such as `LIVE_VIDEO_FRAME = 2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Constant {
    /// An upper-case TypeScript name.
    pub name: String,
    pub description: String,
    pub value: Value,
}

/// A plugin's page (`plugins.md` § The page): its package, and what it
/// reads and calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    /// The page package, such as `@demicodes/plugin-browser`, which the web
    /// app's registry lists and the plugin's types are generated into.
    pub package: String,
    /// Its state for the user, which reaches every page on the sync channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<State>,
    /// Its state for one conversation, which a page reads by revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<State>,
    #[serde(default)]
    pub methods: Vec<Method>,
    /// The work panel kinds whose tabs the backend keeps and the plugin
    /// takes part in (`plugins.md` § Panel kinds).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panel_kinds: Vec<String>,
    /// The topics the plugin itself is told about, with a `topic` request
    /// (`plugins.md` § Topics).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub told: Vec<Topic>,
}

impl Page {
    /// The page package `package`, with no state and no methods yet.
    pub fn new(package: impl Into<String>) -> Self {
        Self {
            package: package.into(),
            user: None,
            conversation: None,
            methods: Vec::new(),
            panel_kinds: Vec::new(),
            told: Vec::new(),
        }
    }

    /// The backend keeps the tabs of the work panel kind `kind`, which the
    /// plugin takes part in.
    pub fn panel_kind(mut self, kind: impl Into<String>) -> Self {
        self.panel_kinds.push(kind.into());
        self
    }

    /// The plugin is told when `topic` fires.
    pub fn told(mut self, topic: Topic) -> Self {
        self.told.push(topic);
        self
    }

    pub fn user_state(mut self, state: State) -> Self {
        self.user = Some(state);
        self
    }

    pub fn conversation_state(mut self, state: State) -> Self {
        self.conversation = Some(state);
        self
    }

    pub fn method(mut self, method: Method) -> Self {
        self.methods.push(method);
        self
    }

    /// The state of `scope`, if the page declares one.
    pub fn state(&self, scope: Scope) -> Option<&State> {
        match scope {
            Scope::User => self.user.as_ref(),
            Scope::Conversation => self.conversation.as_ref(),
        }
    }
}

/// One scope of a page's state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct State {
    pub schema: Schema,
    /// The product changes that mark it changed, each of the state's scope.
    #[serde(default)]
    pub topics: Vec<Topic>,
    /// The operations of a command package a read of it calls. A state one
    /// of whose operations the startup catalog does not serve is left out,
    /// as a method is.
    #[serde(default)]
    pub operations: Vec<NativeOperation>,
}

impl State {
    /// A state that is `S`.
    pub fn new<S: JsonSchema>() -> Self {
        Self {
            schema: schema_of::<S>(Direction::Serializes),
            topics: Vec::new(),
            operations: Vec::new(),
        }
    }

    /// It is read again when `topic` fires.
    pub fn follows(mut self, topic: Topic) -> Self {
        self.topics.push(topic);
        self
    }

    pub fn calls(mut self, operation: NativeOperation) -> Self {
        self.operations.push(operation);
        self
    }
}

/// A product change a page state can follow (`plugins.md` § Topics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    /// The user's exposes: one is created, renewed or destroyed, or the
    /// earliest one expires.
    Exposes,
    /// A job of the conversation ends.
    Jobs,
}

impl Topic {
    /// The scope of the state it marks.
    pub fn scope(self) -> Scope {
        match self {
            Self::Exposes => Scope::User,
            Self::Jobs => Scope::Conversation,
        }
    }
}

/// A page method: what its parameters and result look like, whom it is
/// for, and the package operations it calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Method {
    /// A snake_case word, such as `renew`.
    pub name: String,
    pub scope: Scope,
    pub params: Schema,
    pub result: Schema,
    /// The operations of a command package the method calls. A method one
    /// of whose operations the startup catalog does not serve is left out,
    /// as a command group is.
    #[serde(default)]
    pub operations: Vec<NativeOperation>,
}

impl Method {
    /// The method `name` for `scope`, whose parameters decode into `P` and
    /// whose result is `R`.
    pub fn new<P: JsonSchema, R: JsonSchema>(name: impl Into<String>, scope: Scope) -> Self {
        Self {
            name: name.into(),
            scope,
            params: schema_of::<P>(Direction::Deserializes),
            result: schema_of::<R>(Direction::Serializes),
            operations: Vec::new(),
        }
    }

    pub fn calls(mut self, operation: NativeOperation) -> Self {
        self.operations.push(operation);
        self
    }
}

/// Whom a page method is called for, which decides its route
/// (`web-api.md` § Plugin calls), and whom a page state is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The user: `POST /api/plugins/:plugin/calls/:method`.
    User,
    /// One conversation of the user's:
    /// `POST /api/conversations/:id/plugins/:plugin/calls/:method`.
    Conversation,
}

#[derive(Clone, Copy)]
enum Direction {
    Deserializes,
    Serializes,
}

/// The JSON Schema of `T` as the plugin host validates with it: draft
/// 2020-12, its definitions under `$defs`.
fn schema_of<T: JsonSchema>(direction: Direction) -> Schema {
    let mut settings = SchemaSettings::draft2020_12();
    settings.meta_schema = None;
    let settings = match direction {
        Direction::Deserializes => settings,
        Direction::Serializes => settings.for_serialize(),
    };
    let Value::Object(object) = settings
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
    else {
        unreachable!("a root schema is an object")
    };
    Schema::new(object.into_iter().collect()).expect("schemars writes a schema that compiles")
}

/// One command tree of a plugin, and where it goes in the command set every
/// node starts from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Commands {
    pub placement: Placement,
    /// The tree, as data: an `rpc` leaf reaches the plugin as a command
    /// request, a native leaf runs its operation on the Host.
    pub tree: Node<NativeOperation>,
}

/// Where a plugin's command tree goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// A named group of the `demi` root, for a plugin of this repository.
    Demi,
    /// A root command of its own.
    Root,
}
