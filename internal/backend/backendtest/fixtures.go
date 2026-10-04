package backendtest

import (
	"context"
	"fmt"
	"io"

	"github.com/wspl/demi/internal/plugin"
)

// Pattern makes a backend transfer fixture of length bytes varied by seed,
// repeating only every 64,256 bytes so reads from shifted offsets are visible.
func Pattern(length int, seed byte) []byte {
	const period = 251 * 256
	data := make([]byte, length)
	for i := 0; i < min(period, length); i++ {
		data[i] = byte((i*31+(i>>8))%251) ^ seed
	}
	for offset := period; offset < length; offset += period {
		copy(data[offset:], data[:min(period, length-offset)])
	}
	return data
}

// StreamsPlugin declares user streams bound to a scenario's native fixture.
// It serves the same manifest through the normal plugin registration boundary.
func StreamsPlugin(streams []plugin.Stream) plugin.Factory {
	return &streamsFactory{
		manifest: plugin.Manifest{
			ID:          "fixture",
			Name:        "Fixture streams",
			Description: "The native test fixture's user streams.",
			Streams:     append([]plugin.Stream{}, streams...),
		},
	}
}

// ScriptedMachines runs the scenario machine manager as a program. It accepts
// --artifacts <directory>, prints the socket path as its first output line,
// and serves until input ends or ctx is canceled. It then stops and joins all
// runners. It owns and closes input to unblock its reader during cancellation.
// args excludes argv[0]; the caller handles process signals through ctx.
func ScriptedMachines(
	ctx context.Context,
	args []string,
	input io.ReadCloser,
	output, diagnostics io.Writer,
) (status int) {
	report := func(err error) {
		// A diagnostic write cannot recover a failure of this fixture program.
		_, _ = fmt.Fprintln(diagnostics, err)
		status = 1
	}
	defer func() {
		if input != nil {
			if err := input.Close(); err != nil {
				report(err)
			}
		}
	}()
	if len(args) != 0 && (len(args) != 2 || args[0] != "--artifacts") {
		_, _ = fmt.Fprintln(diagnostics, "Usage: scripted_machines [--artifacts <directory>]")
		return 2
	}
	manager, err := startScriptedManager(ctx)
	if err != nil {
		report(err)
		return status
	}
	defer func() {
		if err := manager.Close(context.WithoutCancel(ctx)); err != nil {
			report(err)
		}
	}()
	if len(args) == 2 {
		manager.SetScript(MachineScript{Artifacts: &args[1]})
	}
	if _, err := fmt.Fprintln(output, manager.Socket()); err != nil {
		report(err)
		return status
	}
	ended := make(chan struct{})
	go func() {
		defer close(ended)
		// Any end of the input read ends service.
		_, _ = io.Copy(io.Discard, input)
	}()
	select {
	case <-ended:
	case <-ctx.Done():
	}
	// Closing the owned input interrupts its blocking Read. Always join that
	// reader before returning, including when a signal ends service first.
	if err := input.Close(); err != nil {
		report(err)
	}
	<-ended
	// The input was closed above; do not close it again in the early-exit defer.
	input = nil
	return status
}

type streamsFactory struct{ manifest plugin.Manifest }

// Manifest returns the fixture stream declarations.
func (f *streamsFactory) Manifest() plugin.Manifest {
	return f.manifest
}

// Instance creates the stateless fixture plugin.
func (*streamsFactory) Instance() plugin.Plugin {
	return noRequests{}
}

type noRequests struct{}

// Call handles the fixture plugin request.
func (noRequests) Call(context.Context, plugin.Request, plugin.Port) (plugin.Reply, error) {
	return nil, &plugin.ErrorFailed{Message: "a plugin of streams only receives no request"}
}
