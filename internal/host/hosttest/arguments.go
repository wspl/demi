package hosttest

//revive:disable:exported
// Contract doc comments below are product text, which does not start with the declared name.

//go:generate go run github.com/wspl/demi/tools/contractgen

// The input of `demi todo add`.
// +demi:schema
// +demi:root
type AddArgs struct {
	// Todo text
	// +demi:length chars max=1
	Text string `json:"text"`
	// How many copies
	Count *uint32 `json:"count,omitempty"`
}

// Reply is the structured result of adding todos.
// +demi:schema
// +demi:root
// +demi:tolerant
type Reply struct {
	Added []string `json:"added"`
}

// Items is the stored todo list used in handler tests.
// +demi:root
type Items []string
