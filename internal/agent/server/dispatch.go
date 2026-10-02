package server

import (
	"context"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

func (c *Connection[H]) attached() *Tree[H] {
	c.server.mu.Lock()
	defer c.server.mu.Unlock()
	tree := c.server.trees[c.root]
	if tree == nil {
		return nil
	}
	if _, ok := tree.attachments[c]; !ok {
		return nil
	}
	return tree
}

func (c *Connection[H]) send(frame framewire.ServerFrame) {
	if !c.outbox.push(frame) {
		c.Detach()
	}
}
func (c *Connection[H]) reject(kind framewire.ClientFrameKind, reason string) {
	c.send(&framewire.RejectedFrame{Command: kind, Reason: reason})
}
func (c *Connection[H]) report(err error) {
	frame := &framewire.ErrorFrame{Message: err.Error()}
	var content *ContentError
	if errors.As(err, &content) {
		frame.Code = content.Code
	}
	c.send(frame)
}
func (c *Connection[H]) steerResult(id core.BlockID, err error) {
	var outcome framewire.SteerOutcome = &framewire.AcceptedSteer{}
	if err != nil {
		outcome = &framewire.RejectedSteer{Reason: err.Error()}
	}
	c.send(&framewire.SteerResultFrame{SteerID: id, Outcome: outcome})
}

func (c *Connection[H]) handle(ctx context.Context, frame framewire.ClientFrame) {
	switch frame.Kind() {
	case framewire.ClientFrameKindOpen:
		c.open(ctx)
		return
	case framewire.ClientFrameKindClose:
		c.closeTree(ctx)
		return
	}
	tree := c.attached()
	if tree == nil {
		switch f := frame.(type) {
		case *framewire.SteerFrame:
			c.steerResult(f.SteerID, session.SteerError("No session is open on this connection"))
		case *framewire.SteerQueuedMessageFrame:
			c.steerResult(f.SteerID, session.SteerError("No session is open on this connection"))
		case *framewire.CancelPendingSteerFrame, *framewire.AbortSubagentsFrame, *framewire.AbortSubagentFrame:
		case *framewire.AbortFrame, *framewire.ClearMessageQueueFrame, *framewire.CloseFrame, *framewire.CompactFrame, *framewire.DequeueMessageFrame, *framewire.EditAndSendFrame, *framewire.OpenFrame, *framewire.ResumeFrame, *framewire.RetryFrame, *framewire.SendFrame, *framewire.SendQueuedMessageFrame, *framewire.ShellAbortFrame, *framewire.ShellWriteFrame, *framewire.SyncTranscriptFrame:
			c.reject(frame.Kind(), "No session is open")
		}
		return
	}
	c.dispatch(ctx, tree, frame)
}

func (c *Connection[H]) open(ctx context.Context) {
	permit, err := c.server.opening.Acquire(ctx, c.root)
	if err != nil {
		c.report(err)
		return
	}
	defer permit.Release()
	if c.attached() != nil {
		c.reject(framewire.ClientFrameKindOpen, "A session is already open on this connection")
		return
	}
	tree, continuation, err := c.server.liveOrOpen(ctx, c.root, c.cwd)
	if err != nil {
		c.report(err)
		return
	}
	tree.attach(c)
	if continuation != nil {
		if err := tree.continueRestored(ctx, *continuation); err != nil {
			c.report(err)
		}
	}
}

func (c *Connection[H]) closeTree(ctx context.Context) {
	permit, err := c.server.opening.Acquire(ctx, c.root)
	if err != nil {
		c.report(err)
		return
	}
	defer permit.Release()
	if tree := c.attached(); tree != nil {
		// Disposal reports its final-save failure to every attachment.
		_ = c.server.disposeTree(context.WithoutCancel(ctx), tree)
		return
	}
	c.send(&framewire.ClosedFrame{})
}

func (c *Connection[H]) dispatch(ctx context.Context, tree *Tree[H], frame framewire.ClientFrame) {
	agent := tree.root.session
	switch f := frame.(type) {
	case *framewire.SendFrame:
		content, media, err := resolveMessage(ctx, c.resolver, f.Content)
		if err != nil {
			c.report(err)
			return
		}
		agent.HoldMedia(&media)
		if _, err := agent.Send(content, f.MessageID); err != nil {
			c.reject(frame.Kind(), err.Error())
		}
	case *framewire.SteerFrame:
		content, media, err := resolveMessage(ctx, c.resolver, f.Content)
		if err == nil {
			agent.HoldMedia(&media)
			err = agent.Steer(content, f.SteerID)
		}
		c.steerResult(f.SteerID, err)
	case *framewire.SteerQueuedMessageFrame:
		found, err := agent.SteerQueuedMessage(f.MessageID, f.SteerID)
		if err == nil && !found {
			err = session.SteerError("Queued message not found")
		}
		c.steerResult(f.SteerID, err)
	case *framewire.CancelPendingSteerFrame:
		agent.CancelPendingSteer(f.SteerID)
	case *framewire.DequeueMessageFrame:
		agent.DequeueMessage(f.MessageID)
	case *framewire.SendQueuedMessageFrame:
		agent.SendQueuedMessage(f.MessageID)
	case *framewire.ClearMessageQueueFrame:
		agent.ClearMessageQueue()
	case *framewire.AbortFrame:
		result, err := agent.Abort(ctx)
		if err == nil {
			err = c.waitPublished(ctx, tree, agent.Transcript().Version.Revision)
		}
		if err != nil {
			c.report(err)
		} else {
			c.send(&framewire.AbortResultFrame{Result: result})
		}
	case *framewire.SyncTranscriptFrame:
		tree.frames.Lock()
		for _, frame := range tree.freshTranscripts(c) {
			c.send(frame)
		}
		tree.frames.Unlock()
	case *framewire.AbortSubagentsFrame:
		if err := tree.abortChildren(ctx, tree.id); err != nil {
			tree.report(err)
		}
	case *framewire.AbortSubagentFrame:
		if tree.isChildOf(f.SubagentID, tree.id) {
			if err := tree.abortChild(ctx, f.SubagentID); err != nil {
				tree.report(err)
			}
		}
	case *framewire.RetryFrame, *framewire.ResumeFrame, *framewire.CompactFrame:
		phase := agent.Phase()
		if phase != core.SessionPhaseIdle {
			c.reject(frame.Kind(), fmt.Sprintf("Session is busy (%s)", phase))
			return
		}
		var err error
		switch frame.Kind() {
		case framewire.ClientFrameKindRetry:
			_, err = agent.Retry()
		case framewire.ClientFrameKindResume:
			_, err = agent.Resume()
		default:
			_, err = agent.Compact()
		}
		if err != nil {
			c.reject(frame.Kind(), err.Error())
		}
	case *framewire.EditAndSendFrame:
		c.edit(ctx, tree, f.Request)
	case *framewire.ShellWriteFrame:
		_, err := tree.shellsOf(f.CommandID).runtime.access.Write(ctx, f.CommandID, f.Stdin)
		if err != nil {
			c.report(err)
		} else {
			c.send(&framewire.ShellWriteResultFrame{CommandID: f.CommandID})
		}
	case *framewire.ShellAbortFrame:
		if _, err := tree.shellsOf(f.CommandID).runtime.access.Abort(ctx, f.CommandID); err != nil {
			c.report(err)
		}
	case *framewire.OpenFrame, *framewire.CloseFrame:
		// Open and close are dispatched before looking up an attachment.
	}
}

// edit registers the reply before admission so acceptance is sent by the node event.
func (c *Connection[H]) edit(ctx context.Context, tree *Tree[H], request framewire.EditRequest) {
	agent := tree.root.session
	digest, err := session.EditDigest(request)
	if err != nil {
		c.send(&framewire.EditResultFrame{OperationID: request.OperationID, Outcome: &framewire.RejectedEdit{Reason: err.Error()}})
		return
	}
	reply := &editReply{operation: request.OperationID, digest: digest}
	tree.frames.Lock()
	c.editReply = reply
	tree.frames.Unlock()
	check, err := agent.CheckEdit(request.OperationID, digest, request.Version)
	var accepted *store.EditReceipt
	if err == nil {
		switch v := check.(type) {
		case *session.EditAccepted:
			accepted = &v.Receipt
		case *session.EditInFlight:
			_, err = v.Acceptance.Wait(ctx)
		case *session.EditProceed:
			var content []session.EditContent
			var media store.HeldMedia
			content, media, err = resolveEdit(ctx, c.resolver, request.Content)
			if err == nil {
				agent.HoldMedia(&media)
				_, err = agent.EditAndSend(ctx, session.EditSubmission{OperationID: request.OperationID, Target: request.TargetBlockID, Version: request.Version, Content: content, Digest: digest})
			}
		}
	}
	tree.frames.Lock()
	defer tree.frames.Unlock()
	if c.editReply != reply {
		return
	}
	c.editReply = nil
	if err != nil {
		c.send(&framewire.EditResultFrame{OperationID: request.OperationID, Outcome: &framewire.RejectedEdit{Reason: err.Error()}})
	} else if accepted != nil {
		c.send(&framewire.EditResultFrame{OperationID: request.OperationID, Outcome: &framewire.AcceptedEdit{TurnID: accepted.TurnID}})
	}
}

type editReply struct {
	operation core.OperationID
	digest    string
}

func (t *Tree[H]) shellsOf(command core.CommandID) *Node[H] {
	if t.root.runtime.access.Environments.Owning(command) != nil {
		return t.root
	}
	for _, child := range t.descendants(t.id) {
		if child.node.runtime.access.Environments.Owning(command) != nil {
			return child.node
		}
	}
	return t.root
}
