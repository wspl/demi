package commandservice

import "errors"

// A Sequence names a sequence of a conversation that a service draws numbers
// from.
type Sequence string

// SequenceTab is the conversation's browser tabs, the only sequence a service
// draws from.
const SequenceTab Sequence = "tab"

// NumbersOpen is the metadata that opens the numbers stream, which carries
// nothing.
//
//demi:wire
type NumbersOpen struct{}

// A NumbersRequest asks for Count numbers of a conversation's sequence. The
// service writes each as one standard output record of the numbers stream. ID
// is the service's own, unique among its requests in flight.
//
//demi:wire
type NumbersRequest struct {
	ID           uint64   `json:"id"`
	Conversation string   `json:"conversation" check:"chars=1..ConversationNameChars,pattern=nameCharacters"`
	Sequence     Sequence `json:"sequence" check:"oneof=tab"`
	Count        int      `json:"count" check:"range=1..MaxNumbers"`
}

// A NumbersAnswer answers the request with the same ID, as one input chunk of
// the numbers stream: the first of its consecutive numbers, or why there are
// none. It carries one of the two.
//
//demi:wire
type NumbersAnswer struct {
	ID    uint64  `json:"id"`
	First *uint64 `json:"first,omitzero" check:"range=1.."`
	Error *string `json:"error,omitzero" check:"chars=1.."`
}

func (a NumbersAnswer) check() error {
	if (a.First == nil) == (a.Error == nil) {
		return errors.New("a numbers answer carries either its first number or its error")
	}
	return nil
}
