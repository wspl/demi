//! The shell frames of a conversation's root (`runtime.md` § Frame
//! protocol): a running shell tool's status reaches the client as
//! `shell_output`, and so do the commands the transcript last saw running
//! that the root still owns, when a client attaches or asks for a fresh
//! transcript.

use demi_agent_protocol::{CommandView, ServerFrame, ShellStatus};
use demi_core::{Block, CommandId, ShellViewStatus, ToolView};
use demi_shell::{CommandState, CommandStatus};

/// The `shell_output` frame of `status`: binary stdout is described by its
/// size, never sent.
pub(crate) fn shell_output(status: &CommandStatus) -> ServerFrame {
    let command = CommandView {
        shell_id: status.shell_id.clone(),
        command_id: status.command_id.clone(),
        output_dir: status.output_dir.clone(),
        stdout: status.stdout.clone(),
        stderr: status.stderr.clone(),
        output: status.output.clone(),
        running_ms: status.running_ms,
        idle_ms: status.idle_ms,
    };
    let status = match &status.state {
        CommandState::Running { hint } => ShellStatus::Running {
            command,
            running_hint: hint.clone(),
        },
        CommandState::Exited {
            exit_code,
            binary_stdout,
        } => ShellStatus::Exited {
            command,
            exit_code: *exit_code,
            binary_stdout: binary_stdout.as_ref().map(|binary| binary.info),
        },
        CommandState::Aborted => ShellStatus::Aborted { command },
    };
    ServerFrame::ShellOutput {
        status: Box::new(status),
    }
}

/// The commands the transcript last saw running. A stored view is history:
/// whether such a command is still alive is its environment's to say.
pub(crate) fn stored_running_commands(blocks: &[Block]) -> Vec<CommandId> {
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
