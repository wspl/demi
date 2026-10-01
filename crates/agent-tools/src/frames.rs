//! The live view of a tree's commands as the pages receive it (`runtime.md`
//! § Live output): each command's `shell_output`, and the commands a node's
//! transcript last saw running, which a page that attaches or asks for a
//! fresh transcript receives with the ones that run.

use demi_conversation_socket_protocol::{CommandView, ServerFrame, ShellStatus};
use demi_host_interface::{PageState, PageView};
use demi_shared_types::{Block, CommandId, NodeId, ShellViewStatus, ToolView};

/// The `shell_output` frame of `view`, a command of the subagent `subagent`,
/// or of the root when none.
pub fn shell_output(subagent: Option<NodeId>, view: PageView) -> ServerFrame {
    let command = CommandView {
        shell_id: view.shell_id,
        command_id: view.command_id,
        tool_use_id: view.tool_use_id,
        tail: view.tail,
        chars: view.chars,
        running_ms: view.running_ms,
    };
    let status = match view.state {
        PageState::Running => ShellStatus::Running { command },
        PageState::Exited { exit_code } => ShellStatus::Exited { command, exit_code },
        PageState::Aborted => ShellStatus::Aborted { command },
    };
    ServerFrame::ShellOutput {
        subagent_id: subagent,
        status: Box::new(status),
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
