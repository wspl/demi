package tools

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
)

// ShellOutput returns a command's shell_output frame. A nil subagent identifies
// the root; otherwise it identifies the subagent whose command is shown.
func ShellOutput(subagent *core.NodeID, view host.PageView) framewire.ServerFrame {
	panic("not written: a-tools")
}

// StoredRunningCommands returns commands the transcript last saw running.
// These views are history; their environments determine whether they are alive.
func StoredRunningCommands(blocks []core.Block) []core.CommandID { panic("not written: a-tools") }
