//! The command context (`native-runtime.md` § Command context): what every
//! declared command of a job or a user stream knows beyond its arguments.
//! The backend is its only source and builds it when the work starts.

use demi_command_service::protocol::{CommandCaller, CommandContext, CommandLocale};
use demi_web_api::ids::{ConversationId, UserId};

use crate::storage::StorageError;
use crate::storage::control::ControlService;

/// The locale commands receive until the user's browser reports one.
pub(crate) fn default_locale() -> CommandLocale {
    CommandLocale {
        time_zone: "UTC".into(),
        languages: vec!["en-US".into()],
    }
}

/// The context of work `caller` starts for `conversation`, whose owner is
/// `user`: the user's reported locale, or the default before one.
pub(crate) async fn command_context(
    control: &ControlService,
    user: &UserId,
    conversation: &ConversationId,
    caller: CommandCaller,
) -> Result<CommandContext, StorageError> {
    let preferences = control.preferences(user.clone()).await?;
    Ok(CommandContext {
        conversation: conversation.to_string(),
        caller,
        locale: preferences.locale.unwrap_or_else(default_locale),
    })
}
