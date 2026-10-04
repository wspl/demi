package types

import "fmt"

// validateAgentMessageBlock enforces the receipt's identity relationship.
func validateAgentMessageBlock(value AgentMessageBlock) error {
	if value.ID() != value.Message.ID {
		return fmt.Errorf("id: must be the message's id, %s", value.Message.ID)
	}
	return nil
}

// validateAgentMessage checks the rules that relate an event to its sender and body.
func validateAgentMessage(value AgentMessage) error {
	switch value.Event.(type) {
	case *MessageEvent:
		if IsBlank(value.Content) {
			return fmt.Errorf("content: an explicit agent message must not be empty")
		}
	case *CompletionEvent:
		expected := (CompletionID{Child: value.Sender.ID, Round: value.Sender.Round}).String()
		if string(value.ID) != expected {
			return fmt.Errorf("id: a completion's id must be %s", expected)
		}
	}
	return nil
}
