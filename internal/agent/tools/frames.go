package tools

import (
	"slices"

	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/types"
)

// ShellOutput returns a command's shell_output frame. A nil subagent identifies
// the root; otherwise it identifies the subagent whose command is shown.
func ShellOutput(subagent *types.NodeID, view host.PageView) conversationproto.ServerFrame {
	command := conversationproto.CommandView{
		ShellID:   view.ShellID,
		CommandID: view.CommandID,
		ToolUseID: view.ToolUseID,
		Tail:      view.Tail,
		Chars:     view.Chars,
		RunningMs: view.RunningMs,
	}
	var status conversationproto.ShellStatus
	switch view.State.Phase {
	case host.Running:
		status = &conversationproto.RunningStatus{CommandView: command}
	case host.Exited:
		status = &conversationproto.ExitedStatus{CommandView: command, ExitCode: view.State.ExitCode}
	case host.Aborted:
		status = &conversationproto.AbortedStatus{CommandView: command}
	}
	return &conversationproto.ShellOutputFrame{SubagentID: subagent, Status: status}
}

// StoredRunningCommands returns commands the transcript last saw running.
// These views are history; their environments determine whether they are alive.
func StoredRunningCommands(blocks []types.Block) []types.CommandID {
	var running []types.CommandID
	for _, block := range blocks {
		call, ok := block.(*types.ToolCallBlock)
		if !ok {
			continue
		}
		view, ok := call.View.(*types.ShellView)
		if !ok {
			continue
		}
		running = slices.DeleteFunc(running, func(id types.CommandID) bool { return id == view.CommandID })
		if view.Status == types.ShellViewStatusRunning {
			running = append(running, view.CommandID)
		}
	}
	return running
}
