//! The command context (`native-runtime.md` § Command context): what every
//! declared command of a job or a user stream knows beyond its arguments.
//! The backend is its only source and builds it when the work starts.

use demi_command_protocol::{ColorScheme, CommandCaller, CommandContext, CommandLocale};
use demi_web_api_protocol::ids::{ConversationId, ProviderId, UserId};

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;

/// The locale commands receive until the user's browser reports one.
pub fn default_locale() -> CommandLocale {
    CommandLocale {
        time_zone: "UTC".into(),
        languages: vec!["en-US".into()],
    }
}

/// The context of work `caller` starts for `conversation`, whose owner is
/// `user`: the user's reported locale, or the default before one.
pub async fn command_context(
    control: &ControlService,
    user: &UserId,
    conversation: &ConversationId,
    caller: CommandCaller,
) -> Result<CommandContext, StorageError> {
    context(control, user, conversation.to_string(), caller).await
}

/// The context of `user`'s work for the provider entry `provider` that
/// belongs to no conversation, such as installing its CLI
/// (`claude-code.md` § The package): it names the entry.
pub async fn provider_context(
    control: &ControlService,
    user: &UserId,
    provider: &ProviderId,
) -> Result<CommandContext, StorageError> {
    context(
        control,
        user,
        format!("provider-{provider}"),
        CommandCaller::User {},
    )
    .await
}

async fn context(
    control: &ControlService,
    user: &UserId,
    scope: String,
    caller: CommandCaller,
) -> Result<CommandContext, StorageError> {
    let Reported {
        locale,
        color_scheme,
    } = reported(control, user).await?;
    Ok(CommandContext {
        conversation: scope,
        caller,
        locale,
        color_scheme,
    })
}

/// The color scheme commands receive until the user's page reports one.
pub const DEFAULT_COLOR_SCHEME: ColorScheme = ColorScheme::Light;

/// What `user`'s commands receive of what their page reported: its
/// browser's locale and the color scheme it shows.
pub struct Reported {
    pub locale: CommandLocale,
    pub color_scheme: ColorScheme,
}

/// What `user`'s page last reported, each part the default before one.
pub async fn reported(control: &ControlService, user: &UserId) -> Result<Reported, StorageError> {
    let preferences = control.preferences(user.clone()).await?;
    Ok(Reported {
        locale: preferences.locale.unwrap_or_else(default_locale),
        color_scheme: preferences.color_scheme.unwrap_or(DEFAULT_COLOR_SCHEME),
    })
}
