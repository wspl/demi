package core

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
)

// The id of a child's completion receipt, `subagent:<child id>:<round>`: it
// names exactly one execution round of one child, so reopening the child
// starts a receipt of its own.
type CompletionID struct {
	Child NodeID
	Round uint64
}

func (id CompletionID) String() string { return fmt.Sprintf("subagent:%s:%d", id.Child, id.Round) }
func (id CompletionID) BlockID() BlockID {
	return BlockID{Identity: Identity[BlockIDKind]{text: id.String()}}
}

func ParseCompletionID(text string) (CompletionID, error) {
	refusal := errors.New("not a completion id (subagent:<child id>:<round>)")
	rest, ok := strings.CutPrefix(text, "subagent:")
	if !ok {
		return CompletionID{}, refusal
	}
	split := strings.LastIndexByte(rest, ':')
	if split < 0 || split == len(rest)-1 {
		return CompletionID{}, refusal
	}
	for _, c := range rest[split+1:] {
		if c < '0' || c > '9' {
			return CompletionID{}, refusal
		}
	}
	round, err := strconv.ParseUint(rest[split+1:], 10, 64)
	if err != nil || round > MaxSafeInteger {
		return CompletionID{}, refusal
	}
	child, err := ParseNodeID(rest[:split])
	if err != nil {
		return CompletionID{}, refusal
	}
	return CompletionID{Child: child, Round: round}, nil
}

func (m AgentMessage) check() error {
	var violations []error
	switch m.Event.(type) {
	case AgentMessageEventCompletion, *AgentMessageEventCompletion:
		expected := CompletionID{Child: m.Sender.ID, Round: m.Sender.Round}
		if m.ID != expected.BlockID() {
			violations = append(violations, &InvalidError{Path: "id", Rule: "a completion's id must name the round it completes"})
		}
	case AgentMessageEventMessage, *AgentMessageEventMessage:
		if IsBlank(m.Content) {
			violations = append(violations, &InvalidError{Path: "content", Rule: "an explicit agent message must not be empty"})
		}
	}
	return errors.Join(violations...)
}
func (b AgentMessageBlock) check() error {
	if b.ID != b.Message.ID {
		return &InvalidError{Path: "id", Rule: "must be the message's id"}
	}
	return nil
}
