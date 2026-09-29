package commandservice

import "github.com/google/jsonschema-go/jsonschema"

// A ConversationOperation is what a [ConversationRequest] asks of a service.
type ConversationOperation string

// The operations of the conversation endpoint.
const (
	// OperationRelease ends everything the service holds for a conversation.
	OperationRelease ConversationOperation = "release"
	// OperationStatus lists the conversations the service holds.
	OperationStatus ConversationOperation = "status"
)

// A ConversationRequest is what a trusted parent sends the conversation
// endpoint: release a conversation's state, or report which conversations hold
// any.
type ConversationRequest struct {
	Operation ConversationOperation `json:"operation"`
	// Conversation names the conversation to release; a status names none.
	Conversation string `json:"conversation,omitzero"`
}

var _ = declare[ConversationRequest](func(s *jsonschema.Schema) {
	limitConversationName(prop(s, "conversation"))
	tagged(s, "operation", string(OperationRelease), string(OperationStatus), form{requires: []string{"conversation"}}, form{refuses: []string{"conversation"}})
}, nil)

// A ConversationStatus is a service's answer to a status request: the
// conversations that hold state in it.
type ConversationStatus struct {
	Conversations []string `json:"conversations"`
}

var _ = declare[ConversationStatus](func(s *jsonschema.Schema) {
	prop(s, "conversations").Items.MinLength = jsonschema.Ptr(1)
}, nil)
