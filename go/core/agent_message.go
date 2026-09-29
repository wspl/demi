package core

//demi:wire
type AgentMessage struct {
	ID          BlockID           `json:"id" check:"func=Validate"`
	Sender      Sender            `json:"sender"`
	RecipientID NodeID            `json:"recipientId" check:"func=Validate"`
	Timestamp   Timestamp         `json:"timestamp" check:"func=Validate"`
	Content     string            `json:"content"`
	Event       AgentMessageEvent `json:"event"`
}

//demi:wire
type Sender struct {
	ID          NodeID `json:"id" check:"func=Validate"`
	Number      uint64 `json:"number" check:"range=..MaxSafeInteger"`
	Description string `json:"description"`
	Round       uint64 `json:"round" check:"range=1..MaxSafeInteger"`
}

//demi:union tag=type
type AgentMessageEvent interface{ isAgentMessageEvent() }

//demi:variant message
type AgentMessageEventMessage struct {
}

func (AgentMessageEventMessage) isAgentMessageEvent() {}

//demi:variant completion
type AgentMessageEventCompletion struct {
	Outcome CompletionOutcome `json:"outcome"`
}

func (AgentMessageEventCompletion) isAgentMessageEvent() {}

//demi:enum
type CompletionOutcome string

const (
	CompletionOutcomeCompleted CompletionOutcome = "completed"
	CompletionOutcomeFailed    CompletionOutcome = "failed"
	CompletionOutcomeAborted   CompletionOutcome = "aborted"
)
