package database

import (
	"github.com/wspl/demi/internal/types"
)

// CommandOutput is a command's row: when it ended, and its output.
type CommandOutput struct {
	Command types.CommandID
	Ended   types.Timestamp
	Output  OutputRow
}
