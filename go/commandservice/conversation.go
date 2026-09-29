package commandservice

// A ConversationRequest is what a trusted parent sends the conversation
// endpoint: release a conversation's state, or report which conversations hold
// any.
//
//demi:union tag=operation
type ConversationRequest interface {
	conversationRequest()
}

// A ReleaseRequest ends everything the service holds for a conversation.
//
//demi:variant release
type ReleaseRequest struct {
	Conversation string `json:"conversation" check:"chars=1..ConversationNameChars,pattern=nameCharacters"`
}

// A StatusRequest lists the conversations the service holds.
//
//demi:variant status
type StatusRequest struct{}

func (ReleaseRequest) conversationRequest() {}
func (StatusRequest) conversationRequest()  {}

// A ConversationStatus is a service's answer to a status request: the
// conversations that hold state in it.
//
//demi:wire
type ConversationStatus struct {
	Conversations []string `json:"conversations" check:"each(chars=1..)"`
}
