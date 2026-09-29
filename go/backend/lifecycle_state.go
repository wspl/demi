package backend

// ConversationWatches are each conversation's idle watch (the lifecycle
// part, G7g).
type ConversationWatches struct{}

// stopAll stops the idle watches; a retirement already running finishes (a
// close step).
func (*ConversationWatches) stopAll() {}
