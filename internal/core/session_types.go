package core

// A message waiting in the queue; the checkpoint keeps the queue. The page
// derives the text it shows from the content.
// +demi:root direction=receive output=protocol
type QueuedMessage struct {
	// The message's id, which becomes its turn's id.
	ID      TurnID             `json:"id"`
	Content []UserContentBlock `json:"content"`
}

// A human steer the session accepted but has not yet written into the
// transcript. It is never saved.
// +demi:tolerant
// +demi:root direction=receive output=protocol
type PendingSteer struct {
	// The steer's id, which its `steer` block will have.
	ID BlockID `json:"id"`
	// The running turn that receives the steer.
	TurnID TurnID `json:"turnId"`
	// The model selection when the steer was accepted.
	Model   ModelSelection     `json:"model"`
	Content []UserContentBlock `json:"content"`
}
