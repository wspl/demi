package server

import (
	"slices"
	"time"

	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

const outputInterval = 250 * time.Millisecond

type waitingOutput struct {
	command core.CommandID
	child   *core.NodeID
	record  *host.CommandRecord
}

type nodeFeed[H host.Host] struct {
	tree  *Tree[H]
	child *core.NodeID
}

func (f *nodeFeed[H]) Watching() (bool, <-chan struct{}) {
	t := f.tree
	t.server.mu.Lock()
	defer t.server.mu.Unlock()
	return len(t.attachments) != 0, t.changed
}

func (f *nodeFeed[H]) Changed(record *host.CommandRecord) {
	t := f.tree
	t.frames.Lock()
	defer t.frames.Unlock()
	view := record.PageView()
	t.server.mu.Lock()
	wasRunning := t.running[view.CommandID]
	if view.State.Phase == host.Running {
		t.running[view.CommandID] = true
	} else {
		delete(t.running, view.CommandID)
	}
	attached := len(t.attachments) != 0
	if attached {
		if view.State.Phase == host.Running {
			if !slices.ContainsFunc(t.waiting, func(w waitingOutput) bool { return w.command == view.CommandID }) {
				t.waiting = append(t.waiting, waitingOutput{command: view.CommandID, child: f.child, record: record})
			}
		} else {
			t.waiting = slices.DeleteFunc(t.waiting, func(w waitingOutput) bool { return w.command == view.CommandID })
		}
	}
	t.server.mu.Unlock()
	if attached && view.State.Phase != host.Running {
		t.publish(tools.ShellOutput(f.child, view))
	}
	if attached || wasRunning != (view.State.Phase == host.Running) {
		t.bump()
	}
}

// sendChanges owns the timer that coalesces running command output.
func (t *Tree[H]) sendChanges() {
	timer := time.NewTimer(outputInterval)
	defer timer.Stop()
	timer.Stop()
	for {
		changed := t.changes()
		t.server.mu.Lock()
		waiting := len(t.waiting) != 0
		due := time.Until(t.sent.Add(outputInterval))
		t.server.mu.Unlock()
		var timeout <-chan time.Time
		if waiting {
			timer.Reset(max(due, 0))
			timeout = timer.C
		}
		select {
		case <-t.ctx.Done():
			return
		case <-changed:
			timer.Stop()
		case <-timeout:
			t.frames.Lock()
			t.server.mu.Lock()
			pending := t.waiting
			t.waiting = nil
			t.sent = time.Now()
			t.server.mu.Unlock()
			for _, item := range pending {
				view := item.record.PageView()
				if view.State.Phase == host.Running {
					t.publish(tools.ShellOutput(item.child, view))
				}
			}
			t.frames.Unlock()
		}
	}
}
