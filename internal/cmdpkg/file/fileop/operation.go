package fileop

import "fmt"

// The package's id, which its release descriptor names and the coding
// agent's commands bind to.
const Package = "demi.file"

// A decoded invocation: the operation and its checked arguments.
//
//sumtype:decl
type Operation interface {
	operation()
}

func (*ReadArgs) operation()   {}
func (*CreateArgs) operation() {}
func (*EditArgs) operation()   {}
func (*PatchArgs) operation()  {}

// Why an invocation could not be decoded.
// An absent Err means the operation name is unknown; otherwise Err is the
// argument decoding or validation failure.
type OperationError struct {
	Name string
	Err  error
}

func (e *OperationError) Error() string {
	if e.Err != nil {
		return e.Err.Error()
	}
	return fmt.Sprintf("unknown operation %s", e.Name)
}

// Unwrap returns the argument failure, or nil for an unknown operation.
func (e *OperationError) Unwrap() error { return e.Err }

// Operations returns the package's operations, as its descriptor lists them.
func Operations() []string {
	return []string{"file.read", "file.create", "file.edit", "file.patch"}
}

// Parse decodes the arguments of the operation named name.
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
		return nil, &OperationError{Name: name}
	}
	if err != nil {
		return nil, &OperationError{Name: name, Err: err}
	}
	return op, nil
}
