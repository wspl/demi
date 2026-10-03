package fileop

import (
	"errors"
	"fmt"
)

// ErrUnknownOperation means the invocation names no operation of this package.
var ErrUnknownOperation = errors.New("unknown operation")

// Package is the package's id, which its release descriptor names and the coding
// agent's commands bind to.
const Package = "demi.file"

// Operation is a decoded invocation: the operation and its checked arguments.
//
//sumtype:decl
type Operation interface {
	operation()
}

func (*ReadArgs) operation()   {}
func (*CreateArgs) operation() {}
func (*EditArgs) operation()   {}
func (*PatchArgs) operation()  {}

// Operations returns the package's operations, as its descriptor lists them.
func Operations() []string {
	return []string{"file.read", "file.create", "file.edit", "file.patch"}
}

// Parse decodes the arguments of the operation named name. An unknown name
// returns an error wrapping ErrUnknownOperation; invalid arguments return the
// decoding error.
func Parse(name string, args []byte) (Operation, error) {
	var op Operation
	var err error
	switch name {
	case "file.read":
		var value ReadArgs
		value, err = DecodeReadArgs(args)
		op = &value
	case "file.create":
		var value CreateArgs
		value, err = DecodeCreateArgs(args)
		op = &value
	case "file.edit":
		var value EditArgs
		value, err = DecodeEditArgs(args)
		op = &value
	case "file.patch":
		var value PatchArgs
		value, err = DecodePatchArgs(args)
		op = &value
	default:
		return nil, fmt.Errorf("%w %s", ErrUnknownOperation, name)
	}
	if err != nil {
		return nil, err
	}
	return op, nil
}
