//! The command context (`crates/command-protocol/src/invocation.rs`): what every
//! declared command of a job or a user stream knows beyond its arguments.
//! The backend is its only source and builds it when the work starts.

use demi_command_protocol::{
    ColorScheme, CommandCaller, CommandContext, CommandLocale,
    PROVIDER_SCOPE_PREFIX,
};
use demi_web_api_protocol::ids::{ConversationId, ProviderId};
use demi_web_api_protocol::settings::BrowserLocale;

use demi_core_store::StorageError;
use demi_core_store::store::CoreStore;

/// The locale commands receive until the user's browser reports one.
pub fn default_locale() -> CommandLocale {
    CommandLocale {
        time_zone: "UTC".into(),
        languages: vec!["en-US".into()],
    }
}

/// The context of work `caller` starts for `conversation`: the reported
/// locale, or the default before one.
pub async fn command_context(
    store: &CoreStore,
    conversation: &ConversationId,
    caller: CommandCaller,
) -> Result<CommandContext, StorageError> {
    context(store, conversation.to_string(), caller).await
}

/// The context of work for the provider entry `provider` that
/// belongs to no conversation, such as installing its CLI
/// (`crates/command-package-claude-code/src/lib.rs`): it names the entry.
pub async fn provider_context(
    store: &CoreStore,
    provider: &ProviderId,
) -> Result<CommandContext, StorageError> {
    context(
        store,
        format!("{PROVIDER_SCOPE_PREFIX}{provider}"),
        CommandCaller::User {},
    )
    .await
}

async fn context(
    store: &CoreStore,
    scope: String,
    caller: CommandCaller,
) -> Result<CommandContext, StorageError> {
    let Reported {
        locale,
        color_scheme,
    } = reported(store).await?;
    Ok(CommandContext {
        conversation: scope,
        caller,
        locale,
        color_scheme,
    })
}

/// The color scheme commands receive until the user's page reports one.
pub const DEFAULT_COLOR_SCHEME: ColorScheme = ColorScheme::Light;

/// What commands receive of what their page reported: its
/// browser's locale and the color scheme it shows.
pub struct Reported {
    pub locale: CommandLocale,
    pub color_scheme: ColorScheme,
}

/// What the page last reported, each part the default before one.
pub async fn reported(store: &CoreStore) -> Result<Reported, StorageError> {
    let preferences = store.preferences().await?;
    Ok(Reported {
        locale: preferences
            .locale
            .map_or_else(default_locale, BrowserLocale::command_locale),
        color_scheme: preferences.color_scheme.unwrap_or(DEFAULT_COLOR_SCHEME),
    })
}
