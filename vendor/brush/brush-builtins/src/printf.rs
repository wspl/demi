use clap::Parser;
use std::{ffi::OsString, io::Write, ops::ControlFlow};
use uucore::format;

use brush_core::{Error, ErrorKind, ExecutionResult, builtins, escape, expansion};

/// Format a string.
#[derive(Parser)]
#[clap(disable_help_flag = true, disable_version_flag = true)]
pub(crate) struct PrintfCommand {
    /// If specified, the output of the command is assigned to this variable.
    #[arg(short = 'v')]
    output_variable: Option<String>,

    /// Format string + arguments to the format string.
    #[arg(trailing_var_arg = true, required = true, allow_hyphen_values = true)]
    format_and_args: Vec<String>,
}

impl builtins::Command for PrintfCommand {
    type Error = brush_core::Error;

    async fn execute<SE: brush_core::ShellExtensions>(
        &self,
        context: brush_core::ExecutionContext<'_, SE>,
    ) -> Result<ExecutionResult, Self::Error> {
        let invocation = invocation(&context)?;
        if let Some(variable_name) = &self.output_variable {
            // Format to a u8 vector.
            let mut result: Vec<u8> = vec![];
            if let Some(ended) = run_in(invocation, &context, || {
                format(self.format_and_args.as_slice(), &mut result)
            })? {
                return Ok(ended);
            }

            // Convert to a string.
            let result_str = String::from_utf8(result).map_err(|_| {
                brush_core::ErrorKind::PrintfInvalidUsage("invalid UTF-8 output".into())
            })?;

            // Assign to the selected variable.
            expansion::assign_to_named_parameter(
                context.shell,
                &context.params,
                variable_name,
                result_str,
            )
            .await?;
        } else {
            let mut stdout = context.stdout();
            if let Some(ended) = run_in(invocation, &context, || {
                format(self.format_and_args.as_slice(), &mut stdout)
            })? {
                return Ok(ended);
            }
            stdout.flush()?;
        }

        Ok(ExecutionResult::success())
    }
}

/// The invocation uucore's formatting runs in, as an embedded utility's does:
/// it reads the locale from the shell's exported variables (`%q`) and writes
/// its warnings (`printf %d abc`) to printf's standard error through the
/// host's control, which records a write to a file, and has nothing to read
/// or write them through outside one.
fn invocation<SE: brush_core::ShellExtensions>(
    context: &brush_core::ExecutionContext<'_, SE>,
) -> Result<uucore::context::Context, brush_core::Error> {
    // A stream the command closed (`printf x 2>&-`) is the null device, which
    // takes what is written and gives nothing to read.
    let stream = |descriptor| -> Result<std::sync::Arc<std::fs::File>, brush_core::Error> {
        let file = match context.try_fd(descriptor) {
            Some(file) => file.into_file()?,
            None => std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(if cfg!(windows) { "NUL" } else { "/dev/null" })?,
        };
        Ok(std::sync::Arc::new(file))
    };
    let shell = &*context.shell;
    Ok(uucore::context::Context {
        name: "printf",
        control: shell
            .execution_host()
            .and_then(|host| host.utility_control(shell.child_attributes())),
        live_input: false,
        // printf creates no file: the mask is the shell's, or none beyond the
        // process's own.
        umask: shell.child_attributes().umask.unwrap_or(0),
        cwd: shell.working_dir().to_owned(),
        env: shell
            .env()
            .iter_exported()
            .filter(|(_, variable)| variable.value().is_set())
            .map(|(name, variable)| (name.clone(), variable.value().to_cow_str(shell).into_owned()))
            .collect(),
        descriptors: std::collections::BTreeMap::new(),
        stdin: stream(0)?,
        stdout: stream(1)?,
        stderr: stream(2)?,
    })
}

/// Runs `work` in `invocation`, or gives the result printf ends with when
/// uucore ends it as it ends a utility: 141 when a diagnostic meets a pipe
/// whose reader has gone, as SIGPIPE ends a program, and the shell's
/// cancellation once its job is cancelled.
fn run_in<SE: brush_core::ShellExtensions>(
    invocation: uucore::context::Context,
    context: &brush_core::ExecutionContext<'_, SE>,
    work: impl FnOnce() -> Result<(), brush_core::Error>,
) -> Result<Option<ExecutionResult>, brush_core::Error> {
    let ended = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        uucore::context::with(invocation, work)
    })) {
        Ok(result) => return result.map(|()| None),
        Err(ended) => ended,
    };
    if let Some(request) = ended.downcast_ref::<uucore::context::ExitRequest>() {
        return Ok(Some(brush_core::ExecutionExitCode::from(request.0 as u8).into()));
    }
    if ended.is::<uucore::context::Cancelled>() {
        context.shell.check_execution()?;
        return Err(std::io::Error::other("shell job cancelled").into());
    }
    std::panic::resume_unwind(ended)
}

fn format(format_and_args: &[String], writer: impl Write) -> Result<(), brush_core::Error> {
    match format_and_args {
        // Special-case invocation of printf with %q-based format string from bash-completion.
        // It has hard-coded expectation of backslash-style escaping instead of quoting.
        [fmt, arg] if fmt == "%q" => format_special_case_for_percent_q(None, arg, writer),
        [fmt, arg] if fmt == "~%q" => format_special_case_for_percent_q(Some("~"), arg, writer),
        // Handle format string with arguments using uucore
        [fmt, args @ ..] => format_via_uucore(fmt, args.iter(), writer),
        // Handle case with no format string (we shouldn't be able to get here since clap will
        // fail parsing when the format string is missing)
        [] => Err(ErrorKind::PrintfInvalidUsage("missing operand".into()).into()),
    }
}

fn format_special_case_for_percent_q(
    prefix: Option<&str>,
    arg: &str,
    mut writer: impl Write,
) -> Result<(), brush_core::Error> {
    let mut result = escape::quote_if_needed(arg, escape::QuoteMode::BackslashEscape).to_string();

    if let Some(prefix) = prefix {
        result.insert_str(0, prefix);
    }

    write!(writer, "{result}")?;

    Ok(())
}

fn format_via_uucore(
    format_string: &str,
    args: impl Iterator<Item = impl Into<OsString>>,
    mut writer: impl Write,
) -> Result<(), brush_core::Error> {
    // Convert string arguments to FormatArgument::Unparsed
    let format_args: Vec<_> = args
        .map(|s| format::FormatArgument::Unparsed(s.into()))
        .collect();

    // Parse format string once.
    let format_items = parse_format_string(format_string)?;

    // Wrap the format arguments.
    let mut format_args_wrapper = format::FormatArguments::new(&format_args);

    // Keep going until we've exhausted all format arguments. Also make sure to run at least once
    // even if there's no format arguments.
    while format_args.is_empty() || !format_args_wrapper.is_exhausted() {
        // Process all format items, in order. We'll bail when we're told to stop.
        for item in &format_items {
            let control_flow = item
                .write(&mut writer, &mut format_args_wrapper)
                .map_err(|e| match e {
                    // Propagate I/O errors directly so they can be handled appropriately
                    format::FormatError::IoError(io_err) => Error::from(io_err),
                    // Wrap other format errors
                    other => Error::from(ErrorKind::PrintfInvalidUsage(std::format!(
                        "printf formatting error: {other}"
                    ))),
                })?;

            if control_flow == ControlFlow::Break(()) {
                break;
            }
        }

        // Start next batch if not exhausted
        if !format_args_wrapper.is_exhausted() {
            format_args_wrapper.start_next_batch();
        }

        if format_args.is_empty() {
            break;
        }
    }

    Ok(())
}

fn parse_format_string(
    format_string: &str,
) -> Result<Vec<format::FormatItem<format::EscapedChar>>, brush_core::Error> {
    let format_items: Result<Vec<_>, _> =
        format::parse_spec_and_escape(format_string.as_bytes()).collect();

    // Observe any errors we encountered along the way.
    let format_items = format_items
        .map_err(|e| ErrorKind::PrintfInvalidUsage(format!("printf parsing error: {e}")))?;

    Ok(format_items)
}

#[cfg(test)]
#[expect(clippy::panic_in_result_fn)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn sprintf_via_uucore(
        format_string: &str,
        args: impl Iterator<Item = impl Into<OsString>>,
    ) -> Result<String> {
        let mut result = vec![];
        format_via_uucore(format_string, args, &mut result)?;

        Ok(String::from_utf8(result)?)
    }

    #[test]
    fn test_basic_sprintf() -> Result<()> {
        assert_eq!(sprintf_via_uucore("%s", std::iter::once(&"xyz"))?, "xyz");
        assert_eq!(sprintf_via_uucore(r"%d\n", std::iter::once(&"1"))?, "1\n");

        Ok(())
    }

    #[test]
    fn test_sprintf_without_args() -> Result<()> {
        let empty: [&str; 0] = [];

        assert_eq!(sprintf_via_uucore("xyz", empty.iter())?, "xyz");
        assert_eq!(sprintf_via_uucore("%s|", empty.iter())?, "|");

        Ok(())
    }

    #[test]
    fn test_sprintf_with_cycles() -> Result<()> {
        assert_eq!(sprintf_via_uucore("%s|", ["x", "y"].iter())?, "x|y|");

        Ok(())
    }
}
