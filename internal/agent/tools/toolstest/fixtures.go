package toolstest

import (
	"context"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/host"
)

// NoHost is the Host type for test products whose agents run no shell tools.
// It also implements tools.HostResolver[*NoHost], refusing Host resolution.
// Its Host methods must never be called: no environment is made on it.
type NoHost struct{}

// Host refuses shell access because this agent runs no shell tools.
func (h *NoHost) Host(_ context.Context, _ tools.NodeContext) (*NoHost, error) {
	return nil, &host.Error{Kind: host.Unavailable, Message: "this agent runs no shell tools"}
}

// Key is unreachable for a product with no shell Host.
func (h *NoHost) Key() host.Key { panic("NoHost operations are unreachable") }

// DefaultCWD is unreachable for a product with no shell Host.
func (h *NoHost) DefaultCWD() string { panic("NoHost operations are unreachable") }

// Identity is unreachable for a product with no shell Host.
func (h *NoHost) Identity() host.Identity { panic("NoHost operations are unreachable") }

// FS is unreachable for a product with no shell Host.
func (h *NoHost) FS() host.FS { panic("NoHost operations are unreachable") }

// Process is unreachable for a product with no shell Host.
func (h *NoHost) Process() host.Process { panic("NoHost operations are unreachable") }

// NoShells is the shell environment factory of a product whose Host is NoHost.
type NoShells struct{}

// Create cannot be reached by an agent that runs no shell tools.
func (s NoShells) Create(_ context.Context, _ tools.EnvironmentScope, _ *NoHost) (host.ShellEnvironment, error) {
	panic("NoHost operations are unreachable")
}

// Field returns a shell tool result's name: value line, such as commandId.
// It panics when the result has no such field.
func Field(result, name string) string {
	for line := range strings.SplitSeq(result, "\n") {
		if value, ok := strings.CutPrefix(line, name+": "); ok {
			return value
		}
	}
	panic(fmt.Sprintf("the result has no %s:\n%s", name, result))
}

// ShownOutput returns displayed output lines, each with a newline, or empty
// text when none are shown. Repeated unfinished lines remain repeated; running
// status, the newest-output note and the next step are excluded.
func ShownOutput(result string) string {
	_, output, ok := strings.Cut(result, "\noutput:\n")
	if !ok {
		return ""
	}
	output, _, _ = strings.Cut(output, "\nnext: ")
	if at := strings.Index(output, " bytes not shown so far; the newest: "); at >= 0 {
		if start := strings.LastIndex(output[:at], "\n[... "); start >= 0 {
			output = output[:start]
		} else {
			output = ""
		}
	}
	return output + "\n"
}

var (
	_ host.Host                              = (*NoHost)(nil)
	_ tools.HostResolver[*NoHost]            = (*NoHost)(nil)
	_ tools.ShellEnvironmentFactory[*NoHost] = NoShells{}
)
