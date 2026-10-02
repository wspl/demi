package process

import (
	"fmt"
	"strings"
)

//go:generate go run ../../../tools/contractgen ./raw_command.go

//revive:disable:exported // Contract descriptions copy the Rust product text verbatim.

// A command line a client forwards to the runner (`commands.md` § External
// command clients): the execution context it runs in, its root command and
// arguments, and whether its input is the job's live terminal. A request
// that breaks these rules is refused as it is read.
// +demi:root
// +demi:strict
// +demi:check validateRawCommand
type RawCommand struct {
	Context string   `json:"context"`
	Root    string   `json:"root"`
	Argv    []string `json:"argv"`
	Live    bool     `json:"live"`
}

//revive:enable:exported

// NewRawCommand makes a request whose context is a context id, whose root is a
// single command name, and whose arguments contain no NUL.
func NewRawCommand(context, root string, argv []string, live bool) (RawCommand, error) {
	if argv == nil {
		argv = []string{}
	}
	request := RawCommand{Context: context, Root: root, Argv: argv, Live: live}
	return request, request.Validate()
}

// validateRawCommand checks the local command's context, root and arguments.
func validateRawCommand(request RawCommand) error {
	valid := len(request.Context) == 32
	for _, b := range []byte(request.Context) {
		hex := b >= '0' && b <= '9' || b >= 'a' && b <= 'f' || b >= 'A' && b <= 'F'
		if !hex {
			valid = false
		}
	}
	if !valid || request.Root == "" || strings.ContainsAny(request.Root, "/\\\x00") {
		return fmt.Errorf("invalid local command request")
	}
	for _, arg := range request.Argv {
		if strings.ContainsRune(arg, 0) {
			return fmt.Errorf("invalid local command request")
		}
	}
	return nil
}
