package tools

import (
	"encoding/json"
	"sync"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
)

// PageChars is the most characters a page of demi shell output takes, so a
// tool result printing one is never cut.
const PageChars = 12_000

// Standard names one of the five tools the model receives.
type Standard string

const (
	// ShellExec starts a script and observes it without imposing a kill deadline.
	ShellExec Standard = "shell_exec"
	// ShellStatus reads status and new output without waiting or writing stdin.
	ShellStatus Standard = "shell_status"
	// ShellWrite writes non-empty stdin to a running foreground command.
	ShellWrite Standard = "shell_write"
	// ShellAbort stops a running foreground command.
	ShellAbort Standard = "shell_abort"
	// Yield ends this turn and schedules a one-shot wakeup.
	Yield Standard = "yield"
)

// Definitions returns the five model-facing definitions in their standard
// order: exec, status, write, abort, yield. Callers must not mutate them.
func Definitions() []provider.ToolDefinition {
	return definitions()
}

var definitions = sync.OnceValue(makeDefinitions)

// makeDefinitions assembles the standard tools once, in model-facing order.
func makeDefinitions() []provider.ToolDefinition {
	return []provider.ToolDefinition{
		{
			Name: string(ShellExec),
			Description: "Start a shell script and observe it for up to timeoutMs. " +
				"timeoutMs is an observation window, not a kill deadline: " +
				"at timeoutMs the command keeps running and a command handle (commandId) is returned. " +
				"Completed short output is returned directly. " +
				"shell_exec never ends the turn or schedules a wakeup on its own.",
			InputSchema: toolSchema(shellExecInputJSONSchema()),
		},
		{
			Name: string(ShellStatus),
			Description: "Read a running command handle status and any new budgeted output preview. " +
				"Does not wait or write stdin.",
			InputSchema: toolSchema(commandInputJSONSchema()),
		},
		{
			Name: string(ShellWrite),
			Description: "Write non-empty stdin to a running foreground command " +
				"and return status with new budgeted output preview. " +
				"Include a newline for line-oriented prompts.",
			InputSchema: toolSchema(shellWriteInputJSONSchema()),
		},
		{
			Name:        string(ShellAbort),
			Description: "Stop a running foreground command by commandId.",
			InputSchema: toolSchema(commandInputJSONSchema()),
		},
		{
			Name:        string(Yield),
			Description: "End this turn and schedule a one-shot wakeup. Does not touch shell commands.",
			InputSchema: toolSchema(yieldInputJSONSchema()),
		},
	}
}

// toolSchema removes the generated title from a model-facing tool schema.
func toolSchema(schema json.RawMessage) json.RawMessage {
	// Every caller passes a generated JSON object, so parsing cannot fail.
	fields, _ := contract.ObjectFields(schema)
	kept := fields[:0]
	for _, field := range fields {
		if field.Name != "title" {
			kept = append(kept, field)
		}
	}
	// Keeping unchanged fields from valid generated JSON cannot fail encoding.
	result, _ := contract.EncodeObject(kept)
	return result
}
