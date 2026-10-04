package server

import (
	"context"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
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

func (c *Connection[H]) send(frame conversationproto.ServerFrame) {
	if !c.outbox.push(frame) {
		c.Detach()
	}
}

func (c *Connection[H]) reject(kind conversationproto.ClientFrameKind, reason string) {
	c.send(&conversationproto.RejectedFrame{Command: kind, Reason: reason})
}

func (c *Connection[H]) report(err error) {
	frame := &conversationproto.ErrorFrame{Message: err.Error()}
	var content *ContentError
	if errors.As(err, &content) {
		frame.Code = content.Code
	}
	c.send(frame)
}

func (c *Connection[H]) steerResult(id types.BlockID, err error) {
	var outcome conversationproto.SteerOutcome = &conversationproto.AcceptedSteer{}
	if err != nil {
		outcome = &conversationproto.RejectedSteer{Reason: err.Error()}
	}
	c.send(&conversationproto.SteerResultFrame{SteerID: id, Outcome: outcome})
}

func (c *Connection[H]) handle(ctx context.Context, frame conversationproto.ClientFrame) {
	switch frame.Kind() {
	case conversationproto.ClientFrameKindOpen:
		c.open(ctx)
		return
	case conversationproto.ClientFrameKindClose:
		c.closeTree(ctx)
		return
	}
	tree := c.attached()
	if tree == nil {
		switch f := frame.(type) {
		case *conversationproto.SteerFrame:
			c.steerResult(f.SteerID, errNoSession)
		case *conversationproto.SteerQueuedMessageFrame:
			c.steerResult(f.SteerID, errNoSession)
		case *conversationproto.CancelPendingSteerFrame,
			*conversationproto.AbortSubagentsFrame,
			*conversationproto.AbortSubagentFrame:
		case *conversationproto.AbortFrame,
			*conversationproto.ClearMessageQueueFrame,
			*conversationproto.CloseFrame,
			*conversationproto.CompactFrame,
			*conversationproto.DequeueMessageFrame,
			*conversationproto.EditAndSendFrame,
			*conversationproto.OpenFrame,
			*conversationproto.ResumeFrame,
			*conversationproto.RetryFrame,
			*conversationproto.SendFrame,
			*conversationproto.SendQueuedMessageFrame,
			*conversationproto.ShellAbortFrame,
			*conversationproto.ShellWriteFrame,
			*conversationproto.SyncTranscriptFrame:
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
		c.reject(conversationproto.ClientFrameKindOpen, "A session is already open on this connection")
		return
	}
	tree, continuation, continues, err := c.server.liveOrOpen(ctx, c.root, c.cwd)
	if err != nil {
		c.report(err)
		return
	}
	tree.attach(c)
	if continues {
		if err := tree.continueRestored(ctx, continuation); err != nil {
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
	c.send(&conversationproto.ClosedFrame{})
}

func (c *Connection[H]) dispatch(ctx context.Context, tree *Tree[H], frame conversationproto.ClientFrame) {
	agent := tree.root.session
	switch f := frame.(type) {
	case *conversationproto.SendFrame:
		c.sendMessage(ctx, agent, f)
	case *conversationproto.SteerFrame:
		c.steer(ctx, agent, f)
	case *conversationproto.SteerQueuedMessageFrame:
		c.steerQueuedMessage(agent, f)
	case *conversationproto.CancelPendingSteerFrame:
		agent.CancelPendingSteer(f.SteerID)
	case *conversationproto.DequeueMessageFrame:
		agent.DequeueMessage(f.MessageID)
	case *conversationproto.SendQueuedMessageFrame:
		agent.SendQueuedMessage(f.MessageID)
	case *conversationproto.ClearMessageQueueFrame:
		agent.ClearMessageQueue()
	case *conversationproto.AbortFrame:
		c.abort(ctx, tree, agent)
	case *conversationproto.SyncTranscriptFrame:
		tree.frames.Lock()
		for _, frame := range tree.freshTranscripts(c) {
			c.send(frame)
		}
		tree.frames.Unlock()
	case *conversationproto.AbortSubagentsFrame:
		if err := tree.abortChildren(ctx, tree.id); err != nil {
			tree.report(err)
		}
	case *conversationproto.AbortSubagentFrame:
		if tree.isChildOf(f.SubagentID, tree.id) {
			if err := tree.abortChild(ctx, f.SubagentID); err != nil {
				tree.report(err)
			}
		}
	case *conversationproto.RetryFrame, *conversationproto.ResumeFrame, *conversationproto.CompactFrame:
		c.startAction(agent, frame)
	case *conversationproto.EditAndSendFrame:
		c.edit(ctx, tree, f.Request)
	case *conversationproto.ShellWriteFrame:
		c.shellWrite(ctx, tree, f)
	case *conversationproto.ShellAbortFrame:
		if _, err := tree.shellsOf(f.CommandID).runtime.access.Abort(ctx, f.CommandID); err != nil {
			c.report(err)
		}
	case *conversationproto.OpenFrame, *conversationproto.CloseFrame:
		// Open and close are dispatched before looking up an attachment.
	}
}

// edit registers the reply before admission so acceptance is sent by the node event.
func (c *Connection[H]) edit(ctx context.Context, tree *Tree[H], request conversationproto.EditRequest) {
	agent := tree.root.session
	digest, err := session.EditDigest(request)
	if err != nil {
		c.send(
			&conversationproto.EditResultFrame{
				OperationID: request.OperationID,
				Outcome:     &conversationproto.RejectedEdit{Reason: err.Error()},
			},
		)
		return
	}
	reply := &editReply{operation: request.OperationID, digest: digest}
	tree.frames.Lock()
	c.editReply = reply
	tree.frames.Unlock()
	check, err := agent.CheckEdit(ctx, request.OperationID, digest, request.Version)
	var accepted *store.EditReceipt
	if err == nil {
		switch v := check.(type) {
		case *session.EditAccepted:
			accepted = &v.Receipt
		case *session.EditInFlight:
			// Accepted; its EditCommitted event answered this request.
		case *session.EditProceed:
			var content []session.EditContent
			var media store.HeldMedia
			content, media, err = resolveEdit(ctx, c.resolver, request.Content)
			if err == nil {
				agent.HoldMedia(&media)
				_, err = agent.EditAndSend(
					ctx,
					session.EditSubmission{
						OperationID: request.OperationID,
						Target:      request.TargetBlockID,
						Version:     request.Version,
						Content:     content,
						Digest:      digest,
					},
				)
			}
		}
	}
	c.finishEdit(tree, request, reply, accepted, err)
}

type editReply struct {
	operation types.OperationID
	digest    string
}

func (t *Tree[H]) shellsOf(command types.CommandID) *Node[H] {
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

func (c *Connection[H]) sendMessage(ctx context.Context, agent *session.Session, f *conversationproto.SendFrame) {
	content, media, err := resolveMessage(ctx, c.resolver, f.Content)
	if err != nil {
		c.report(err)
		return
	}
	agent.HoldMedia(&media)
	if _, err := agent.Send(content, f.MessageID); err != nil {
		c.reject(f.Kind(), err.Error())
	}
}

func (c *Connection[H]) abort(ctx context.Context, tree *Tree[H], agent *session.Session) {
	result, err := agent.Abort(ctx)
	if err == nil {
		err = c.waitPublished(ctx, tree, agent.Transcript().Version.Revision)
	}
	if err != nil {
		c.report(err)
	} else {
		c.send(&conversationproto.AbortResultFrame{Result: result})
	}
}

func (c *Connection[H]) startAction(agent *session.Session, frame conversationproto.ClientFrame) {
	phase := agent.Phase()
	if phase != types.SessionPhaseIdle {
		c.reject(frame.Kind(), fmt.Sprintf("Session is busy (%s)", phase))
		return
	}
	var err error
	switch frame.Kind() {
	case conversationproto.ClientFrameKindRetry:
		_, err = agent.Retry()
	case conversationproto.ClientFrameKindResume:
		_, err = agent.Resume()
	default:
		_, err = agent.Compact()
	}
	if err != nil {
		c.reject(frame.Kind(), err.Error())
	}
}

func (c *Connection[H]) finishEdit(
	tree *Tree[H],
	request conversationproto.EditRequest,
	reply *editReply,
	accepted *store.EditReceipt,
	err error,
) {
	tree.frames.Lock()
	defer tree.frames.Unlock()
	if c.editReply != reply {
		return
	}
	c.editReply = nil
	if err != nil {
		c.send(
			&conversationproto.EditResultFrame{
				OperationID: request.OperationID,
				Outcome:     &conversationproto.RejectedEdit{Reason: err.Error()},
			},
		)
	} else if accepted != nil {
		c.send(
			&conversationproto.EditResultFrame{
				OperationID: request.OperationID,
				Outcome:     &conversationproto.AcceptedEdit{TurnID: accepted.TurnID},
			},
		)
	}
}

func (c *Connection[H]) steerQueuedMessage(agent *session.Session, f *conversationproto.SteerQueuedMessageFrame) {
	found, err := agent.SteerQueuedMessage(f.MessageID, f.SteerID)
	if err == nil && !found {
		err = errQueuedMessageNotFound
	}
	c.steerResult(f.SteerID, err)
}

func (c *Connection[H]) steer(ctx context.Context, agent *session.Session, f *conversationproto.SteerFrame) {
	content, media, err := resolveMessage(ctx, c.resolver, f.Content)
	if err == nil {
		agent.HoldMedia(&media)
		err = agent.Steer(content, f.SteerID)
	}
	c.steerResult(f.SteerID, err)
}

func (c *Connection[H]) shellWrite(ctx context.Context, tree *Tree[H], f *conversationproto.ShellWriteFrame) {
	_, err := tree.shellsOf(f.CommandID).runtime.access.Write(ctx, f.CommandID, f.Stdin)
	if err != nil {
		c.report(err)
	} else {
		c.send(&conversationproto.ShellWriteResultFrame{CommandID: f.CommandID})
	}
}
