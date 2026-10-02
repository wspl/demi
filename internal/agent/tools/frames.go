package tools

import (
	"slices"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
)

// ShellOutput returns a command's shell_output frame. A nil subagent identifies
// the root; otherwise it identifies the subagent whose command is shown.
func ShellOutput(subagent *core.NodeID, view host.PageView) framewire.ServerFrame {
	command := framewire.CommandView{ShellID: view.ShellID, CommandID: view.CommandID, ToolUseID: view.ToolUseID, Tail: view.Tail, Chars: view.Chars, RunningMs: view.RunningMs}
	var status framewire.ShellStatus
	switch view.State.Phase {
	case host.Running:
		status = &framewire.RunningStatus{CommandView: command}
	case host.Exited:
		status = &framewire.ExitedStatus{CommandView: command, ExitCode: view.State.ExitCode}
	case host.Aborted:
		status = &framewire.AbortedStatus{CommandView: command}
	}
	return &framewire.ShellOutputFrame{SubagentID: subagent, Status: status}
}

// StoredRunningCommands returns commands the transcript last saw running.
// These views are history; their environments determine whether they are alive.
func StoredRunningCommands(blocks []core.Block) []core.CommandID {
	var running []core.CommandID
	for _, block := range blocks {
		call, ok := block.(*core.ToolCallBlock)
		if !ok {
			continue
		}
		view, ok := call.View.(*core.ShellView)
		if !ok {
			continue
		}
		running = slices.DeleteFunc(running, func(id core.CommandID) bool { return id == view.CommandID })
		if view.Status == core.ShellViewStatusRunning {
			running = append(running, view.CommandID)
		}
	}
	return running
}
