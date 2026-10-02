package cmdpkgstest

import (
	"encoding/json"

	"github.com/wspl/demi/internal/commandwire"
)

//go:generate go run ../../../../tools/contractgen .

// fixtureArgs retains arbitrary fixture arguments, as Rust's JSON value does.
// +demi:root
type fixtureArgs map[string]json.RawMessage

// numberAnswer reports the first reserved number.
// +demi:root
type numberAnswer struct {
	First uint64 `json:"first"`
}

// whereAnswer exposes the invocation context for native integration tests.
// +demi:root
type whereAnswer struct {
	Label   json.RawMessage            `json:"label"`
	Context commandwire.CommandContext `json:"context"`
	Cwd     string                     `json:"cwd"`
	// +demi:nullable
	Value *string `json:"value"`
}

// releaseAnswer acknowledges a conversation release.
// +demi:root
type releaseAnswer struct{}
