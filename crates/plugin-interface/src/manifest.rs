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
    "The Demi platform command: every subcommand is a platform domain (file, todo, …).";

/// A plugin's id, such as `todo`: 1 to 32 lowercase letters, digits and
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
    /// Its name in settings, such as `Todo list`.
    pub name: String,
    /// What it does, in one sentence, which settings show.
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

/// A user stream: a page connects to `operation` on the conversation's main
/// Host for as long as it keeps the stream open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stream {
    /// The name the page opens it by, taken once among all plugins.
    pub name: String,
    pub operation: NativeOperation,
}

/// What a plugin's page reads and calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    /// The schema of the plugin's state for the user's pages; none for a
    /// page that only calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Schema>,
    /// The product changes the state is read from: each marks the plugin's
    /// part of the user's product state as changed.
    #[serde(default)]
    pub follows: Vec<Follows>,
    #[serde(default)]
    pub methods: Vec<Method>,
}

impl Page {
    /// A page with no state and no methods yet.
    pub fn new() -> Self {
        Self {
            state: None,
            follows: Vec::new(),
            methods: Vec::new(),
        }
    }

    /// The page's state is `S`.
    pub fn state<S: JsonSchema>(mut self) -> Self {
        self.state = Some(schema_of::<S>(Direction::Serializes));
        self
    }

    pub fn follows(mut self, follows: Follows) -> Self {
        self.follows.push(follows);
        self
    }

    pub fn method(mut self, method: Method) -> Self {
        self.methods.push(method);
        self
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new()
    }
}

/// A product change a plugin's page state follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Follows {
    /// The user's exposes: one is created, renewed or destroyed, or the
    /// earliest one expires.
    Exposes,
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
/// (`web-api.md` § Plugin calls).
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
