//! What a viewer's page receives: the live view's writer
//! (`demi_live_view::writer`), carrying the browser's messages.

use demi_command_package_browser_protocol::live::LiveModuleMessage;

pub(crate) type Writer = demi_live_view::writer::Writer<LiveModuleMessage>;
