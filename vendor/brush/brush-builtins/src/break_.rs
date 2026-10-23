use std::io::Write;

use clap::Parser;

use brush_core::{ExecutionControlFlow, ExecutionExitCode, ExecutionResult, builtins};

/// Breaks out of a control-flow loop.
#[derive(Parser)]
pub(crate) struct BreakCommand {
    /// If specified, indicates which nested loop to break out of.
    #[clap(default_value_t = 1)]
    which_loop: i8,
}

impl builtins::Command for BreakCommand {
    type Error = brush_core::Error;

    async fn execute<SE: brush_core::ShellExtensions>(
        &self,
        context: brush_core::ExecutionContext<'_, SE>,
    ) -> Result<brush_core::ExecutionResult, Self::Error> {
        // Outside any loop it is a warning and nothing more, as in bash, so a script that
        // breaks out of a block it thinks is a loop (autoconf's configure) goes on.
        if !context.shell.in_loop() {
            writeln!(
                context.stderr(),
                "break: only meaningful in a `for', `while', or `until' loop"
            )?;
            return Ok(ExecutionResult::success());
        }

        // If specified, which_loop needs to be positive.
        if self.which_loop <= 0 {
            return Ok(ExecutionExitCode::InvalidUsage.into());
        }

        let mut result = ExecutionResult::success();

        result.next_control_flow = ExecutionControlFlow::BreakLoop {
            #[expect(clippy::cast_sign_loss)]
            levels: (self.which_loop - 1) as usize,
        };

        Ok(result)
    }
}
