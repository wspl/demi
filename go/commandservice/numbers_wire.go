package commandservice

import (
	"errors"

	"github.com/google/jsonschema-go/jsonschema"
)

// A Sequence names a sequence of a conversation that a service draws numbers
// from.
type Sequence string

// SequenceTab is the conversation's browser tabs, the only sequence a service
// draws from.
const SequenceTab Sequence = "tab"

// NumbersOpen is the metadata that opens the numbers stream, which carries
// nothing.
type NumbersOpen struct{}

var _ = declare[NumbersOpen](nil, nil)

// A NumbersRequest asks for Count numbers of a conversation's sequence. The
// service writes each as one standard output record of the numbers stream. ID
// is the service's own, unique among its requests in flight.
type NumbersRequest struct {
	ID           uint64   `json:"id"`
	Conversation string   `json:"conversation"`
	Sequence     Sequence `json:"sequence"`
	Count        int      `json:"count"`
}

var _ = declare[NumbersRequest](func(s *jsonschema.Schema) {
	limitConversationName(prop(s, "conversation"))
	prop(s, "sequence").Enum = []any{string(SequenceTab)}
	count := prop(s, "count")
	count.Minimum = jsonschema.Ptr(1.0)
	count.Maximum = jsonschema.Ptr(float64(MaxNumbers))
}, nil)

// A NumbersAnswer answers the request with the same ID, as one input chunk of
// the numbers stream: the first of its consecutive numbers, or why there are
// none. It carries one of the two.
type NumbersAnswer struct {
	ID    uint64  `json:"id"`
	First *uint64 `json:"first,omitzero"`
	Error *string `json:"error,omitzero"`
}

var _ = declare(func(s *jsonschema.Schema) {
	prop(s, "first").Minimum = jsonschema.Ptr(1.0)
	prop(s, "error").MinLength = jsonschema.Ptr(1)
}, func(v *NumbersAnswer) error {
	if (v.First == nil) == (v.Error == nil) {
		return errors.New("a numbers answer carries either its first number or its error")
	}
	return nil
})
