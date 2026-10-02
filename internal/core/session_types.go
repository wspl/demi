package core

// QueuedMessage holds a user message waiting to start a turn.
// +demi:root direction=receive output=protocol
type QueuedMessage struct {
	ID      TurnID             `json:"id"`
	Content []UserContentBlock `json:"content"`
}

// PendingSteer holds an accepted human steer not yet written to the transcript.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type PendingSteer struct {
	ID      BlockID            `json:"id"`
	TurnID  TurnID             `json:"turnId"`
	Model   ModelSelection     `json:"model"`
	Content []UserContentBlock `json:"content"`
}
