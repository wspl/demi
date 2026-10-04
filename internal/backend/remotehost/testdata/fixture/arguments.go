// Package fixture declares the native and callback test boundaries.
package fixture

import "github.com/wspl/demi/internal/commandproto"

//go:generate go run github.com/wspl/demi/tools/contractgen
//go:generate go run github.com/wspl/demi/tools/contractgen -ts -ts-dir zod .

// The input of `todo add` and `todo note`.
// +demi:schema
// +demi:root direction=receive output=web
type TodoArgs struct {
	// Todo text
	Text string `json:"text"`
}

// The input of `probe hold`.
// +demi:schema
// +demi:root
type HoldArgs struct {
	// Milliseconds to wait
	Ms uint64 `json:"ms"`
}

// The input of `probe json emit`.
// +demi:schema
// +demi:root direction=receive output=web
type EmitArgs struct {
	// The text to print
	Text string `json:"text"`
}

// What `probe json emit --json` declares it prints.
// +demi:tolerant
// +demi:schema
// +demi:root direction=receive output=web
type Emitted struct {
	Ok bool `json:"ok"`
}

// The input of the fixture's `where`, which only its schema declares: the
// native operation reads it.
// +demi:schema
// +demi:root direction=receive output=web
type WhereArgs struct {
	// A label the operation reports
	Label *string `json:"label,omitempty"`
}

// WhereReport validates the native fixture's reported invocation context.
// +demi:root
type WhereReport struct {
	// +demi:nullable
	Label   *string              `json:"label"`
	Context commandproto.Context `json:"context"`
	CWD     string               `json:"cwd"`
	// +demi:nullable
	Value *string `json:"value"`
}
