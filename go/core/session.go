package core

// A message waiting in the queue; the checkpoint keeps the queue. The page
// derives the text it shows from the content.
//
//demi:wire
type QueuedMessage struct {
	// The message's id, which becomes its turn's id.
	ID      TurnID             `json:"id" check:"func=Validate"`
	Content []UserContentBlock `json:"content"`
}

// A human steer the session accepted but has not yet written into the
// transcript. It is never saved.
//
//demi:wire open
type PendingSteer struct {
	// The steer's id, which its `steer` block will have.
	ID BlockID `json:"id" check:"func=Validate"`
	// The running turn that receives the steer.
	TurnID TurnID `json:"turnId" check:"func=Validate"`
	// The model selection when the steer was accepted.
	Model   ModelSelection     `json:"model"`
	Content []UserContentBlock `json:"content"`
}

// What a session is doing, as clients see it.
//
//demi:enum
//demi:export
type SessionPhase string

const (
	SessionPhaseIdle       SessionPhase = "idle"
	SessionPhaseRunning    SessionPhase = "running"
	SessionPhaseCompacting SessionPhase = "compacting"
)
