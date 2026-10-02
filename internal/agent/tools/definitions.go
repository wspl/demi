package tools

import "github.com/wspl/demi/internal/provider"

// PageChars is the most characters a page of demi shell output takes, so a
// tool result printing one is never cut.
const PageChars = 12_000

// StandardTool names one of the five tools the model receives.
type StandardTool string

const (
	// ShellExec starts a script and observes it without imposing a kill deadline.
	ShellExec StandardTool = "shell_exec"
	// ShellStatus reads status and new output without waiting or writing stdin.
	ShellStatus StandardTool = "shell_status"
	// ShellWrite writes non-empty stdin to a running foreground command.
	ShellWrite StandardTool = "shell_write"
	// ShellAbort stops a running foreground command.
	ShellAbort StandardTool = "shell_abort"
	// Yield ends this turn and schedules a one-shot wakeup.
	Yield StandardTool = "yield"
)

// Definitions returns the five model-facing definitions in their standard
// order: exec, status, write, abort, yield. Callers must not mutate them.
func Definitions() []provider.ToolDefinition { panic("not written: a-tools") }
