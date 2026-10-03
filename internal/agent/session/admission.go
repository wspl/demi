package session

import (
	"context"
	"reflect"
	"slices"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

func (s *Session) admit(kind actionKind, content []core.UserContentBlock, id core.TurnID) (*ActionHandle, error) {
	var answer *ActionHandle
	var err error
	s.mutate(func(c *coreState) {
		if err = c.admissionLocked(); err != nil {
			return
		}
		if kind == sendAction {
			duplicate := c.log.HasUserTurn(id) || (c.active != nil && c.active.turn == id)
			for _, a := range c.queue {
				duplicate = duplicate || (a.kind == sendAction && a.turn == id)
			}
			if duplicate {
				answer = newAction()
				c.effects = append(c.effects, func() { answer.finish(Duplicate, nil) })
				return
			}
		} else {
			id = core.TurnID(s.deps.IDs.NextID())
		}
		answer = newAction()
		c.queue = append(c.queue, &action{kind: kind, content: content, turn: id, answer: answer})
		s.startNextLocked()
	})
	return answer, err
}

func (s *Session) steer(content []core.UserContentBlock, id core.BlockID) error {
	var err error
	s.mutate(func(c *coreState) {
		if err = c.steerableLocked(); err != nil {
			return
		}
		c.inputs = append(
			c.inputs,
			pendingInput{steer: &core.PendingSteer{ID: id, TurnID: c.active.turn, Model: c.model, Content: content}},
		)
		c.arrivals++
	})
	return err
}

func (s *Session) acceptAgentMessage(ctx context.Context, message core.AgentMessage) error {
	if err := message.Validate(); err != nil {
		return &AgentMessageError{Kind: AgentMessageInvalid, Detail: err.Error(), Cause: err}
	}
	var err error
	s.mutate(func(c *coreState) {
		refuse := func(kind AgentMessageErrorKind) { err = &AgentMessageError{Kind: kind} }
		if message.RecipientID != c.id {
			refuse(AgentMessageRecipient)
			return
		}
		if c.disposing {
			refuse(AgentMessageClosed)
			return
		}
		if c.preparingEditLocked() {
			refuse(AgentMessageEditing)
			return
		}
		existing, conflict := c.agentMessageLocked(message.ID)
		if existing != nil {
			if !reflect.DeepEqual(*existing, message) {
				refuse(AgentMessageDifferentContent)
				return
			}
			s.startNextLocked()
			return
		}
		if conflict {
			refuse(AgentMessageConflict)
			return
		}
		turn := core.TurnID(message.ID)
		if c.active != nil {
			turn = c.active.turn
		}
		c.inputs = append(
			c.inputs,
			pendingInput{agent: &store.PendingAgentInput{TurnID: turn, Model: c.model, Message: message}},
		)
		c.arrivals++
		c.dirty = true
		s.startNextLocked()
	})
	if err != nil {
		return err
	}
	if err = s.Flush(ctx); err != nil {
		return &AgentMessageError{Kind: AgentMessageStore, Cause: err}
	}
	return nil
}

type inputSelection uint8

const (
	allInputs inputSelection = iota
	exceptAgentMessages
	humanSteers
)

// writeInputsLocked moves the selected inputs into the session's current turn.
func (s *Session) writeInputsLocked(take inputSelection) bool {
	c := &s.core
	agents := false
	kept := []pendingInput{}
	for _, input := range c.inputs {
		if (take == exceptAgentMessages && input.agent != nil) || (take == humanSteers && input.steer == nil) {
			kept = append(kept, input)
			continue
		}
		if input.steer != nil {
			v := input.steer
			c.log.PushSteer(v.ID, c.active.turn, v.Model, v.Content)
		}
		if input.agent != nil {
			v := input.agent
			c.log.PushAgentMessage(c.active.turn, v.Model, v.Message)
			agents = true
			c.dirty = true
		}
		if input.wakeup != nil {
			c.log.PushWakeup(core.BlockID(input.wakeup.ID), c.active.turn, c.model, "steer")
			c.dirty = true
		}
	}
	c.inputs = kept
	return agents
}

func (s *Session) writeInputs(ctx context.Context) error {
	agents := false
	s.mutate(func(_ *coreState) { agents = s.writeInputsLocked(allInputs) })
	if agents {
		return s.Flush(ctx)
	}
	return nil
}

func (s *Session) cancelPendingSteer(id core.BlockID) bool {
	removed := false
	s.mutate(func(c *coreState) {
		for i, input := range c.inputs {
			if input.steer != nil && input.steer.ID == id {
				c.inputs = slices.Delete(c.inputs, i, i+1)
				c.arrivals--
				removed = true
				break
			}
		}
	})
	return removed
}

// writeInputsSince writes input only when arrivals increased across a round or call.
func (s *Session) writeInputsSince(ctx context.Context, arrivals uint64) error {
	s.mu.Lock()
	added := s.core.arrivals > arrivals
	s.mu.Unlock()
	if added {
		return s.writeInputs(ctx)
	}
	return nil
}

// agentMessageLocked finds duplicate or conflicting input while the session mutex is held.
func (c *coreState) agentMessageLocked(id core.BlockID) (*core.AgentMessage, bool) {
	var existing *core.AgentMessage
	conflict := false
	for _, input := range c.inputs {
		if input.agent != nil && input.agent.Message.ID == id {
			existing = &input.agent.Message
		}
		if input.steer != nil && input.steer.ID == id {
			conflict = true
		}
		if input.wakeup != nil && core.BlockID(input.wakeup.ID) == id {
			conflict = true
		}
	}
	if block := c.log.Find(id); block != nil {
		if b, ok := block.(*core.AgentMessageBlock); ok {
			existing = &b.Message
		} else {
			conflict = true
		}
	}
	return existing, conflict
}
