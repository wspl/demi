package database

import (
	"github.com/wspl/demi/internal/core"
)

// CommandOutput is a command's row: when it ended, and its output.
type CommandOutput struct {
	Command core.CommandID
	Ended   core.Timestamp
	Output  OutputRow
}
