package core

// A message between agents. The supervisor supplies the sender from the
// invoking node, so a model cannot impersonate another sender.
//
//demi:wire
type AgentMessage struct {
	// The id of the `agent_message` block the message becomes. A
	// completion's id is its [`CompletionId`].
	ID          BlockID `json:"id" check:"func=Validate"`
	Sender      Sender  `json:"sender"`
	RecipientID NodeID  `json:"recipientId" check:"func=Validate"`
	// When the message was sent; for a completion, when the child closed.
	Timestamp Timestamp `json:"timestamp" check:"func=Validate"`
	// The body: a completion's result for `completed`, otherwise its failure
	// text, which can be empty. An explicit message is never empty.
	Content string            `json:"content"`
	Event   AgentMessageEvent `json:"event"`
}

// Who sent an agent message.
//
//demi:wire
type Sender struct {
	ID NodeID `json:"id" check:"func=Validate"`
	// The number the model knows the sender by: 0 for the root
	// (`runtime.md` § Identifiers the model sees).
	Number uint64 `json:"number" check:"range=..MaxSafeInteger"`
	// The sender's description; `root session` for the root.
	Description string `json:"description"`
	// The sender's round: 1 for its first run, one more at each resume.
	Round uint64 `json:"round" check:"range=1..MaxSafeInteger"`
}

// What an agent message is.
//
//demi:union tag=type
//demi:export
type AgentMessageEvent interface{ isAgentMessageEvent() }

// An explicit communication between live agents.
//
//demi:variant message
type AgentMessageEventMessage struct {
}

func (AgentMessageEventMessage) isAgentMessageEvent() {}

// A supervisor's receipt of a child's end.
//
//demi:variant completion
type AgentMessageEventCompletion struct {
	Outcome CompletionOutcome `json:"outcome"`
}

func (AgentMessageEventCompletion) isAgentMessageEvent() {}

// How a child ended.
//
//demi:enum
//demi:export
type CompletionOutcome string

const (
	CompletionOutcomeCompleted CompletionOutcome = "completed"
	CompletionOutcomeFailed    CompletionOutcome = "failed"
	CompletionOutcomeAborted   CompletionOutcome = "aborted"
)
