package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

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
	panic("not written: r-process")
}

// validateRawCommand checks the local command's context, root and arguments.
func validateRawCommand(request RawCommand) error { panic("not written: r-process") }
