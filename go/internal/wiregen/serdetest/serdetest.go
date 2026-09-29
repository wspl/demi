// Package serdetest declares wire types that use what serde gave the Rust:
// members of a struct that are ignored when unknown, a required member that may
// be null, an adjacently tagged union, a union flattened into a struct, and a
// wire struct of another package. The tests exercise their generated code.
package serdetest

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"encoding/json/jsontext"

	"github.com/wspl/demi/go/runnerproto"
)

// A Note tolerates members it does not have, and has a member that is required
// and may be null.
//
//demi:wire
//demi:open
type Note struct {
	Title string  `json:"title" check:"chars=1.."`
	Reset *string `json:"reset" check:"nullable"`
	Later *int    `json:"later,omitzero"`
}

// A Command is an operation and its parameters, told apart by the member op, with
// the parameters in the member params.
//
//demi:union tag=op content=params
type Command interface{ command() }

// Stop takes no parameters, and ignores members it does not know.
//
//demi:variant stop
//demi:open
type Stop struct{}

// Start names a device and, once it is up, its boot record.
//
//demi:variant start
//demi:open
type Start struct {
	Device string                  `json:"device" check:"chars=1.."`
	Boot   runnerproto.ManagedBoot `json:"boot" check:"func=runnerproto.ValidateManagedBoot"`
	Limit  *uint8                  `json:"limit,omitzero"`
}

// Resize is strict: its parameters have no members but its own.
//
//demi:variant resize
type Resize struct {
	Bytes uint64 `json:"bytes" check:"range=1.."`
}

func (Stop) command()   {}
func (Start) command()  {}
func (Resize) command() {}

// A Request has an id and the members of a command, as one JSON object.
//
//demi:wire
//demi:open
type Request struct {
	ID   string  `json:"id" check:"chars=1.."`
	Call Command `json:",inline"`
	Tag  *string `json:"tag,omitzero"`
}

// A Holder holds a command as a member of its own, an object with op and params.
//
//demi:wire
type Holder struct {
	Command Command `json:"command"`
}

// A Fleet holds wire structs of another package.
//
//demi:wire
type Fleet struct {
	Boots []runnerproto.ManagedBoot          `json:"boots" check:"each(func=runnerproto.ValidateManagedBoot)"`
	ByID  map[string]runnerproto.ManagedBoot `json:"byId" check:"each(func=bootIsLocal)"`
	Raw   jsontext.Value                     `json:"raw"`
}

// bootIsLocal is a rule of this package that a value of another package's type
// has.
func bootIsLocal(boot runnerproto.ManagedBoot) error {
	return runnerproto.ValidateManagedBoot(boot)
}
