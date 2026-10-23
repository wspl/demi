//! The live view of a tree's commands as the pages receive it
//! (`crates/agent-server/src/server/tree/live.rs`).
//!
//! It holds each command's `shell_output`, and the commands a node's transcript
//! last saw running. A page reads a node's live commands with its latest page.

use demi_conversation_socket_protocol::{CommandView, ServerFrame, ShellStatus};
use demi_host_interface::{PageState, PageView};
use demi_shared_types::{Block, CommandId, NodeId, ShellViewStatus, ToolView};

/// The `shell_output` frame of `view`, a command of the node `node`.
pub fn shell_output(node: NodeId, view: PageView) -> ServerFrame {
    ServerFrame::ShellOutput {
        node,
        status: Box::new(shell_status(view)),
    }
}

/// The status of the command `view` shows, as a page holds it: the answer of
/// a node's latest page for each live command, and the frame's content.
pub fn shell_status(view: PageView) -> ShellStatus {
    let command = CommandView {
        command_id: view.command_id,
        tool_use_id: view.tool_use_id,
        title: view.title,
        script: view.script.to_string(),
        service: view.service,
        tail: view.tail,
        chars: view.chars,
        running_ms: view.running_ms,
    };
    match view.state {
        PageState::Running => ShellStatus::Running {
            command,
            stopping: false,
            waiting_for: None,
        },
        PageState::Waiting { host } => ShellStatus::Running {
            command,
            stopping: false,
            waiting_for: Some(host),
        },
        PageState::Stopping => ShellStatus::Running {
            command,
            stopping: true,
            waiting_for: None,
        },
        PageState::Exited { exit_code } => ShellStatus::Exited { command, exit_code },
        PageState::Aborted => ShellStatus::Aborted { command },
        PageState::Lost { reason } => ShellStatus::Lost { command, reason },
    }
}

/// The commands the transcript last saw running. A stored view is history:
/// whether such a command is still alive is its environment's to say.
pub fn stored_running_commands(blocks: &[Block]) -> Vec<CommandId> {
    let mut running: Vec<CommandId> = Vec::new();
    for block in blocks {
        let Block::ToolCall(call) = block else {
            continue;
        };
        let Some(ToolView::Shell(view)) = &call.view else {
            continue;
        };
        running.retain(|command| *command != view.command_id);
        if view.status == ShellViewStatus::Running {
            running.push(view.command_id.clone());
        }
    }
    running
}
