//! The Rust types the web app receives or sends, which the emitter starts
//! from, the output each belongs to, and what the web app does with each
//! (`contracts.md` § Generated TypeScript). Every type they refer to is
//! emitted with them.

use demi_command_package_browser_protocol::live;
use demi_conversation_socket_protocol as frames;
use demi_shared_types as core;
use demi_web_api_protocol as api;
use schemars::{JsonSchema, Schema, SchemaGenerator};

/// What the web app does with a type, which decides whether its schema is a
/// tolerant or a strict object (`contracts.md` § Strict and tolerant
/// objects).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// The web app receives it, in a frame, a response or a table.
    Receives,
    /// Only the web app sends it; the backend or a native program receives
    /// it.
    Sends,
}

/// A type the emitter starts from.
pub struct Root {
    pub direction: Direction,
    /// Adds the type's schema, and those it refers to, to the generator and
    /// answers the reference to it.
    pub register: fn(&mut SchemaGenerator) -> Schema,
}

fn receives<T: JsonSchema>() -> Root {
    Root {
        direction: Direction::Receives,
        register: SchemaGenerator::subschema_for::<T>,
    }
}

fn sends<T: JsonSchema>() -> Root {
    Root {
        direction: Direction::Sends,
        register: SchemaGenerator::subschema_for::<T>,
    }
}

/// `@demicodes/protocol`: core's types, the conversation socket's frames
/// and the live view's messages. Core's types are listed whether or not a
/// frame refers to them, so that they are declared here and `web` imports
/// them.
pub fn protocol() -> Vec<Root> {
    vec![
        receives::<frames::ServerFrame>(),
        sends::<frames::ClientFrame>(),
        receives::<core::Block>(),
        receives::<core::ProviderModelList>(),
        receives::<core::AuthState>(),
        receives::<core::RuntimeState>(),
        receives::<core::AccountInfo>(),
        receives::<core::LoginPending>(),
        receives::<core::QuotaSnapshot>(),
        receives::<core::WireApi>(),
        receives::<core::PreviewType>(),
        receives::<live::LiveModuleMessage>(),
        sends::<live::LiveViewerMessage>(),
    ]
}

/// `packages/web/src/api/generated`: every REST request and response body.
pub fn web() -> Vec<Root> {
    use api::{
        attachments, auth, cloud, conversations, devices, drafts, error, files, hosts, panel,
        plugins, providers, settings, sidebar, state, usage, users, workspaces,
    };
    vec![
        receives::<error::ErrorBody>(),
        receives::<auth::Identity>(),
        receives::<auth::SetupStatus>(),
        sends::<auth::SetupRequest>(),
        sends::<auth::Credentials>(),
        sends::<auth::NicknamePatch>(),
        sends::<auth::PasswordChange>(),
        sends::<auth::EmailChangeStart>(),
        receives::<auth::EmailChangeStarted>(),
        sends::<auth::EmailChangeConfirm>(),
        receives::<state::ProductState>(),
        receives::<state::SyncEvent>(),
        receives::<settings::Settings>(),
        receives::<settings::UserPreferences>(),
        sends::<settings::PreferencesPatch>(),
        sends::<plugins::PluginSwitch>(),
        sends::<providers::CreateProvider>(),
        sends::<providers::ProviderPatch>(),
        receives::<providers::ProviderAnswer>(),
        receives::<providers::Providers>(),
        receives::<providers::ProviderDetails>(),
        sends::<providers::QuotaRequest>(),
        receives::<providers::QuotaAnswer>(),
        sends::<providers::TestRequest>(),
        receives::<providers::TestResult>(),
        receives::<providers::VendorCatalog>(),
        sends::<providers::SetupTokenImport>(),
        sends::<providers::AddToken>(),
        receives::<providers::AddedAccount>(),
        receives::<providers::Accounts>(),
        sends::<providers::ActivateAccount>(),
        receives::<providers::ActiveAccount>(),
        sends::<providers::SubscriptionLogin>(),
        receives::<providers::LoginStarted>(),
        receives::<providers::LoginAnswer>(),
        receives::<providers::ModelCatalog>(),
        receives::<providers::ProviderCli>(),
        receives::<providers::CliInstallAnswer>(),
        receives::<devices::Devices>(),
        sends::<devices::Claim>(),
        receives::<devices::ClaimedDevice>(),
        receives::<devices::DeviceLog>(),
        receives::<cloud::CloudStatus>(),
        sends::<cloud::CloudReset>(),
        receives::<cloud::CloudResetAnswer>(),
        sends::<conversations::CreateConversation>(),
        receives::<conversations::ConversationAnswer>(),
        receives::<conversations::Conversations>(),
        sends::<conversations::ReadRequest>(),
        receives::<conversations::Transcript>(),
        sends::<conversations::ConversationPatch>(),
        receives::<conversations::ConversationUpdate>(),
        sends::<conversations::ConversationBatch>(),
        receives::<conversations::BatchAnswer>(),
        sends::<conversations::ForkRequest>(),
        receives::<conversations::ForkAnswer>(),
        receives::<attachments::AttachmentAnswer>(),
        receives::<hosts::AttachedHosts>(),
        sends::<hosts::AttachHost>(),
        sends::<hosts::RenameHost>(),
        receives::<files::Directory>(),
        sends::<files::CreateDirectory>(),
        sends::<files::CreateDeviceDirectory>(),
        receives::<files::CreatedDirectory>(),
        receives::<files::FileText>(),
        receives::<files::WorkingTreeChanges>(),
        receives::<files::ChangeSides>(),
        receives::<usage::UsageTotals>(),
        receives::<usage::InstanceUsage>(),
        receives::<users::Users>(),
        sends::<users::CreateUser>(),
        receives::<users::CreatedUser>(),
        sends::<users::PasswordReset>(),
        receives::<workspaces::Workspaces>(),
        sends::<workspaces::CreateWorkspace>(),
        sends::<workspaces::RenameWorkspace>(),
        receives::<workspaces::WorkspaceAnswer>(),
        sends::<sidebar::SidebarReorder>(),
        receives::<panel::WorkPanel>(),
        sends::<drafts::DraftSave>(),
        sends::<drafts::ReplacedDraftAction>(),
        receives::<drafts::DraftAnswer>(),
    ]
}

/// Each plugin page package's generated directory, with the types of its
/// plugin's state, parameters and results (`plugins.md` § The page).
pub fn plugins() -> Vec<(&'static str, Vec<Root>)> {
    use demi_plugin_browser::page as browser;
    use demi_plugin_expose::page as expose;
    vec![
        (
            "packages/plugin-browser/src/generated",
            vec![
                receives::<browser::BrowserTabs>(),
                sends::<browser::OpenTab>(),
                receives::<browser::OpenedTab>(),
                sends::<browser::CloseTab>(),
                sends::<browser::NavigateTab>(),
                sends::<browser::TabHistory>(),
            ],
        ),
        (
            "packages/plugin-expose/src/generated",
            vec![
                receives::<expose::ExposeState>(),
                sends::<expose::ExposeCall>(),
            ],
        ),
    ]
}
