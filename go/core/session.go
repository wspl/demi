package core

//demi:wire
type QueuedMessage struct {
	ID      TurnID             `json:"id" check:"func=Validate"`
	Content []UserContentBlock `json:"content"`
}

//demi:wire open
type PendingSteer struct {
	ID      BlockID            `json:"id" check:"func=Validate"`
	TurnID  TurnID             `json:"turnId" check:"func=Validate"`
	Model   ModelSelection     `json:"model"`
	Content []UserContentBlock `json:"content"`
}

//demi:enum
type SessionPhase string

const (
	SessionPhaseIdle       SessionPhase = "idle"
	SessionPhaseRunning    SessionPhase = "running"
	SessionPhaseCompacting SessionPhase = "compacting"
)
