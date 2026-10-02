package contracts

import (
	"fmt"
	"strings"
	"unicode"
)

// validateAgentMessageBlock enforces the receipt's identity relationship.
func validateAgentMessageBlock(value AgentMessageBlock) error {
	if value.ID != value.Message.ID {
		return fmt.Errorf("id: must be the message's id, %s", value.Message.ID)
	}
	return nil
}

// validateAgentMessage checks the rules that relate an event to its sender and body.
func validateAgentMessage(value AgentMessage) error {
	switch value.Event.(type) {
	case *MessageEvent:
		blank := strings.TrimFunc(value.Content, func(r rune) bool { return r == '\ufeff' || (r != '\u0085' && unicode.IsSpace(r)) }) == ""
		if blank {
			return fmt.Errorf("content: an explicit agent message must not be empty")
		}
	case *CompletionEvent:
		expected := fmt.Sprintf("subagent:%s:%d", value.Sender.ID, value.Sender.Round)
		if string(value.ID) != expected {
			return fmt.Errorf("id: a completion's id must be %s", expected)
		}
	}
	return nil
}
