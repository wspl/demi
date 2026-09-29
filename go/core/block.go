package core

// A message the user submitted, with the harness's text for the turn.
//
//demi:wire
type UserBlock struct {
	ID BlockID `json:"id" check:"func=Validate"`
	// The message's id, which starts the turn.
	TurnID    TurnID         `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	// The content as submitted.
	Content []UserContentBlock `json:"content"`
	// The harness's text for the turn, which the model receives before the
	// content; null when the harness adds none.
	Preamble *string `json:"preamble" check:"nullable"`
}

// The execution context the node's next request runs in, which the model
// receives as a user message.
//
//demi:wire
type ContextBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	TurnID    TurnID         `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Text      string         `json:"text"`
}

// A fired yield wakeup. The model receives the fixed wakeup text as a user
// message or as a steer, as the placement says.
//
//demi:wire
type WakeupBlock struct {
	ID        BlockID         `json:"id" check:"func=Validate"`
	TurnID    TurnID          `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp       `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection  `json:"model"`
	Placement WakeupPlacement `json:"placement"`
}

// A human steer, written at a continuation boundary. Its id is the steer's.
//
//demi:wire
type SteerBlock struct {
	ID        BlockID            `json:"id" check:"func=Validate"`
	TurnID    TurnID             `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp          `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection     `json:"model"`
	Content   []UserContentBlock `json:"content"`
}

// A message from another agent of the tree. Its id is the message's.
//
//demi:wire
type AgentMessageBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	TurnID    TurnID         `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Message   AgentMessage   `json:"message"`
}

// The turn continues after a cut; the model receives "Continue from where
// you left off."
//
//demi:wire
type ResumeBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	TurnID    TurnID         `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
}

// The stopped marker. The user sees it until the turn is continued.
//
//demi:wire
type AbortBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	// Set once the stopped turn is continued.
	IsResumed bool `json:"isResumed"`
}

// Reasoning text, with the vendor's signature when it signed it. A signed
// block is replayed whole.
//
//demi:wire
type ThinkingBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Text      string         `json:"text"`
	Signature *string        `json:"signature" check:"nullable"`
}

// Opaque reasoning data, replayed whole and never shown.
//
//demi:wire
type RedactedThinkingBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Data      string         `json:"data"`
}

// Assistant text.
//
//demi:wire
type TextBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Text      string         `json:"text"`
	// Set once the text is complete and a Fork may start after it
	// (`conversation-fork.md`); omitted until then.
	Forkable bool `json:"forkable,omitzero"`
}

// A tool call the provider requested, completed by the session with its
// result.
//
//demi:wire
type ToolCallBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	// The provider's id of the call.
	ToolUseID string `json:"toolUseId"`
	ToolName  string `json:"toolName"`
	// The call's input as the JSON text the provider supplied, which need
	// not be valid JSON.
	Input  string         `json:"input"`
	Status ToolCallStatus `json:"status"`
	// The result, once the call completed.
	Output []ToolResultContentBlock `json:"output"`
	View   *ToolView                `json:"view" check:"nullable"`
}

// The usage of one completed provider request, which anchors the context
// estimate.
//
//demi:wire
type ResponseBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Usage     TokenUsage     `json:"usage"`
}

// A failed request or an interrupted turn (`failures-and-recovery.md`
// § The failure record).
//
//demi:wire
type ErrorBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Message   string         `json:"message"`
	// The failure's code, such as `rate_limit` or `interrupted`; null when
	// it has none.
	Code *string `json:"code" check:"nullable"`
	// The provider's record of the failure, when a provider failed.
	Diagnostics *ProviderErrorDiagnostics `json:"diagnostics,omitzero"`
}

// Compaction's summary of the history before it, inserted where the kept
// history begins.
//
//demi:wire
type CompactionBoundaryBlock struct {
	ID            BlockID        `json:"id" check:"func=Validate"`
	CreatedAt     Timestamp      `json:"createdAt" check:"func=Validate"`
	Model         ModelSelection `json:"model"`
	Summary       string         `json:"summary"`
	SummaryTokens uint64         `json:"summaryTokens" check:"range=..MaxSafeInteger"`
}

// Compaction's estimate of the size of what it summarized.
//
//demi:wire
type CompactionMarkerBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	// The boundary this pass inserted.
	BoundaryID      BlockID `json:"boundaryId" check:"func=Validate"`
	CompactedTokens uint64  `json:"compactedTokens" check:"range=..MaxSafeInteger"`
}

// One block of a transcript.
//
//demi:union tag=type
//demi:export
type Block interface{ isBlock() }

// A message the user submitted: the only block a user can edit.
//
//demi:variant user
type BlockUser struct {
	UserBlock
}

func (BlockUser) isBlock() {}

// The conversation's execution context changed since the node last saw
// it, such as a target switch; hidden from the user.
//
//demi:variant context
type BlockContext struct {
	ContextBlock
}

func (BlockContext) isBlock() {}

// A yield wakeup fired; hidden from the user.
//
//demi:variant wakeup
type BlockWakeup struct {
	WakeupBlock
}

func (BlockWakeup) isBlock() {}

// A human steer of the running turn.
//
//demi:variant steer
type BlockSteer struct {
	SteerBlock
}

func (BlockSteer) isBlock() {}

// A message from another agent of the tree.
//
//demi:variant agent_message
type BlockAgentMessage struct {
	AgentMessageBlock
}

func (BlockAgentMessage) isBlock() {}

// A turn continues after a cut: a resume, compaction inside a turn, or a
// model switch that landed inside a turn and compacted.
//
//demi:variant resume
type BlockResume struct {
	ResumeBlock
}

func (BlockResume) isBlock() {}

// The stopped marker a Stop leaves.
//
//demi:variant abort
type BlockAbort struct {
	AbortBlock
}

func (BlockAbort) isBlock() {}

//demi:variant thinking
type BlockThinking struct {
	ThinkingBlock
}

func (BlockThinking) isBlock() {}

//demi:variant redacted_thinking
type BlockRedactedThinking struct {
	RedactedThinkingBlock
}

func (BlockRedactedThinking) isBlock() {}

//demi:variant text
type BlockText struct {
	TextBlock
}

func (BlockText) isBlock() {}

//demi:variant tool_call
type BlockToolCall struct {
	ToolCallBlock
}

func (BlockToolCall) isBlock() {}

// The usage of one completed provider request.
//
//demi:variant response
type BlockResponse struct {
	ResponseBlock
}

func (BlockResponse) isBlock() {}

// A failed request or an interrupted turn.
//
//demi:variant error
type BlockError struct {
	ErrorBlock
}

func (BlockError) isBlock() {}

// Compaction's summary, where the kept history begins.
//
//demi:variant compaction_boundary
type BlockCompactionBoundary struct {
	CompactionBoundaryBlock
}

func (BlockCompactionBoundary) isBlock() {}

// Compaction's estimate of what it summarized, at the end.
//
//demi:variant compaction_marker
type BlockCompactionMarker struct {
	CompactionMarkerBlock
}

func (BlockCompactionMarker) isBlock() {}

// Where a fired wakeup entered the transcript.
//
//demi:enum
//demi:export
type WakeupPlacement string

const (
	WakeupPlacementNewTurn WakeupPlacement = "new_turn"
	WakeupPlacementSteer   WakeupPlacement = "steer"
)

// Where a tool call is.
//
//demi:enum
//demi:export
type ToolCallStatus string

const (
	ToolCallStatusCompleted ToolCallStatus = "completed"
	ToolCallStatusError     ToolCallStatus = "error"
	ToolCallStatusExecuting ToolCallStatus = "executing"
)
