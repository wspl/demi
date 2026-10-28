//! Workspaces: the parts of a user's account, each with conversations and
//! projects of its own, which the sidebar shows one at a time. Everything in
//! Settings is the account's, the same in every workspace.

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::ids::{ConversationId, ProjectId, WorkspaceId};
use crate::text::Trimmed;

/// The icon of a workspace's tile, one of the ones the page draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceIcon {
    User,
    Briefcase,
    House,
    Code,
    Rocket,
    Flask,
    Book,
    Palette,
    GraduationCap,
    Heart,
    Star,
    Globe,
    Building,
    Sparkles,
    Music,
    Camera,
    Leaf,
    Dumbbell,
    Plane,
    Gamepad,
    Coffee,
    Wallet,
    Lightbulb,
    Users,
}

impl WorkspaceIcon {
    /// Every icon, in the order the picker shows them and new workspaces
    /// take them.
    pub const ALL: [Self; 24] = [
        Self::User,
        Self::Briefcase,
        Self::House,
        Self::Code,
        Self::Rocket,
        Self::Flask,
        Self::Book,
        Self::Palette,
        Self::GraduationCap,
        Self::Heart,
        Self::Star,
        Self::Globe,
        Self::Building,
        Self::Sparkles,
        Self::Music,
        Self::Camera,
        Self::Leaf,
        Self::Dumbbell,
        Self::Plane,
        Self::Gamepad,
        Self::Coffee,
        Self::Wallet,
        Self::Lightbulb,
        Self::Users,
    ];
}

/// The color of a workspace's tile, one of the ones the page paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceColor {
    Blue,
    Purple,
    Green,
    Amber,
    Red,
    Pink,
    Teal,
    Slate,
}

impl WorkspaceColor {
    /// Every color, in the order the picker shows them and new workspaces
    /// take them.
    pub const ALL: [Self; 8] =
        [Self::Blue, Self::Purple, Self::Green, Self::Amber, Self::Red, Self::Pink, Self::Teal, Self::Slate];
}

/// A workspace as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDto {
    pub id: WorkspaceId,
    /// Its name; absent while it keeps the one an account starts with,
    /// which the page says in its language.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "unwrap_or_skip")]
    #[schemars(with = "String")]
    pub name: Option<String>,
    pub icon: WorkspaceIcon,
    pub color: WorkspaceColor,
}

/// `GET /workspaces`: the user's workspaces, in their order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Workspaces {
    pub workspaces: Vec<WorkspaceDto>,
}

/// `{ workspace }`: the answer of a creation and a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceAnswer {
    pub workspace: WorkspaceDto,
}

/// `POST /workspaces`: a new workspace, last in the order, with the next
/// icon and color. Its name is the page's default for it, in the page's
/// language, which the user changes afterwards.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateWorkspace {
    #[garde(length(chars, min = 1))]
    pub name: Trimmed,
}

/// `PATCH /workspaces/:id`: each field replaces its value.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeWorkspace {
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "Trimmed")]
    #[garde(length(chars, min = 1))]
    pub name: Option<Trimmed>,
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "WorkspaceIcon")]
    #[garde(skip)]
    pub icon: Option<WorkspaceIcon>,
    #[serde(default, with = "unwrap_or_skip")]
    #[schemars(with = "WorkspaceColor")]
    #[garde(skip)]
    pub color: Option<WorkspaceColor>,
}

/// `PUT /workspaces/order`: every workspace of the user's, in its new
/// order.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceOrder {
    #[garde(length(min = 1))]
    pub ids: Vec<WorkspaceId>,
}

/// `POST /workspaces/:id/moves`: conversations and projects of the user's
/// other workspaces, which move into this one. A conversation in a project
/// joins this workspace's project of the same directory, made when there is
/// none; a project joins this workspace's of its directory, merging into it,
/// or moves whole when there is none.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceMove {
    #[serde(default)]
    #[garde(skip)]
    pub conversations: Vec<ConversationId>,
    #[serde(default)]
    #[garde(skip)]
    pub projects: Vec<ProjectId>,
}
