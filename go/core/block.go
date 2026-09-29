package core

//demi:wire
type UserBlock struct {
	ID        BlockID            `json:"id" check:"func=Validate"`
	TurnID    TurnID             `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp          `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection     `json:"model"`
	Content   []UserContentBlock `json:"content"`
	Preamble  *string            `json:"preamble" check:"nullable"`
}

//demi:wire
type ContextBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	TurnID    TurnID         `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Text      string         `json:"text"`
}

//demi:wire
type WakeupBlock struct {
	ID        BlockID         `json:"id" check:"func=Validate"`
	TurnID    TurnID          `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp       `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection  `json:"model"`
	Placement WakeupPlacement `json:"placement"`
}

//demi:wire
type SteerBlock struct {
	ID        BlockID            `json:"id" check:"func=Validate"`
	TurnID    TurnID             `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp          `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection     `json:"model"`
	Content   []UserContentBlock `json:"content"`
}

//demi:wire
type AgentMessageBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	TurnID    TurnID         `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Message   AgentMessage   `json:"message"`
}

//demi:wire
type ResumeBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	TurnID    TurnID         `json:"turnId" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
}

//demi:wire
type AbortBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	IsResumed bool           `json:"isResumed"`
}

//demi:wire
type ThinkingBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Text      string         `json:"text"`
	Signature *string        `json:"signature" check:"nullable"`
}

//demi:wire
type RedactedThinkingBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Data      string         `json:"data"`
}

//demi:wire
type TextBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Text      string         `json:"text"`
	Forkable  bool           `json:"forkable,omitzero"`
}

//demi:wire
type ToolCallBlock struct {
	ID        BlockID                  `json:"id" check:"func=Validate"`
	CreatedAt Timestamp                `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection           `json:"model"`
	ToolUseID string                   `json:"toolUseId"`
	ToolName  string                   `json:"toolName"`
	Input     string                   `json:"input"`
	Status    ToolCallStatus           `json:"status"`
	Output    []ToolResultContentBlock `json:"output"`
	View      *ToolView                `json:"view" check:"nullable"`
}

//demi:wire
type ResponseBlock struct {
	ID        BlockID        `json:"id" check:"func=Validate"`
	CreatedAt Timestamp      `json:"createdAt" check:"func=Validate"`
	Model     ModelSelection `json:"model"`
	Usage     TokenUsage     `json:"usage"`
}

//demi:wire
type ErrorBlock struct {
	ID          BlockID                   `json:"id" check:"func=Validate"`
	CreatedAt   Timestamp                 `json:"createdAt" check:"func=Validate"`
	Model       ModelSelection            `json:"model"`
	Message     string                    `json:"message"`
	Code        *string                   `json:"code" check:"nullable"`
	Diagnostics *ProviderErrorDiagnostics `json:"diagnostics,omitzero"`
}

//demi:wire
type CompactionBoundaryBlock struct {
	ID            BlockID        `json:"id" check:"func=Validate"`
	CreatedAt     Timestamp      `json:"createdAt" check:"func=Validate"`
	Model         ModelSelection `json:"model"`
	Summary       string         `json:"summary"`
	SummaryTokens uint64         `json:"summaryTokens" check:"range=..MaxSafeInteger"`
}

//demi:wire
type CompactionMarkerBlock struct {
	ID              BlockID        `json:"id" check:"func=Validate"`
	CreatedAt       Timestamp      `json:"createdAt" check:"func=Validate"`
	Model           ModelSelection `json:"model"`
	BoundaryID      BlockID        `json:"boundaryId" check:"func=Validate"`
	CompactedTokens uint64         `json:"compactedTokens" check:"range=..MaxSafeInteger"`
}

//demi:union tag=type
type Block interface{ isBlock() }

//demi:variant user
type BlockUser struct {
	UserBlock
}

func (BlockUser) isBlock() {}

//demi:variant context
type BlockContext struct {
	ContextBlock
}

func (BlockContext) isBlock() {}

//demi:variant wakeup
type BlockWakeup struct {
	WakeupBlock
}

func (BlockWakeup) isBlock() {}

//demi:variant steer
type BlockSteer struct {
	SteerBlock
}

func (BlockSteer) isBlock() {}

//demi:variant agent_message
type BlockAgentMessage struct {
	AgentMessageBlock
}

func (BlockAgentMessage) isBlock() {}

//demi:variant resume
type BlockResume struct {
	ResumeBlock
}

func (BlockResume) isBlock() {}

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

//demi:variant response
type BlockResponse struct {
	ResponseBlock
}

func (BlockResponse) isBlock() {}

//demi:variant error
type BlockError struct {
	ErrorBlock
}

func (BlockError) isBlock() {}

//demi:variant compaction_boundary
type BlockCompactionBoundary struct {
	CompactionBoundaryBlock
}

func (BlockCompactionBoundary) isBlock() {}

//demi:variant compaction_marker
type BlockCompactionMarker struct {
	CompactionMarkerBlock
}

func (BlockCompactionMarker) isBlock() {}

//demi:enum
type WakeupPlacement string

const (
	WakeupPlacementNewTurn WakeupPlacement = "new_turn"
	WakeupPlacementSteer   WakeupPlacement = "steer"
)

//demi:enum
type ToolCallStatus string

const (
	ToolCallStatusExecuting ToolCallStatus = "executing"
	ToolCallStatusCompleted ToolCallStatus = "completed"
	ToolCallStatusError     ToolCallStatus = "error"
)
