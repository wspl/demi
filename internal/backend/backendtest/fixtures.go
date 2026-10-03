package backendtest

//revive:disable:unused-parameter

import (
	"context"
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
	return &streamsFactory{manifest: plugin.Manifest{ID: "fixture", Name: "Fixture streams", Description: "The native test fixture's user streams.", Streams: append([]plugin.Stream{}, streams...)}}
}

// ScriptedMachines runs the scenario machine manager as a program. It accepts
// --artifacts <directory>, prints the socket path as its first output line,
// and serves until input ends or ctx is canceled. It then stops and joins all
// runners. It owns and closes input to unblock its reader during cancellation.
// args excludes argv[0]; the caller handles process signals through ctx.
func ScriptedMachines(ctx context.Context, args []string, input io.ReadCloser, output, diagnostics io.Writer) int {
	panic("not written: b-backend")
}

type streamsFactory struct{ manifest plugin.Manifest }

func (f *streamsFactory) Manifest() plugin.Manifest { return f.manifest }
func (*streamsFactory) Instance() plugin.Plugin     { return noRequests{} }

type noRequests struct{}

func (noRequests) Call(context.Context, plugin.Request, plugin.Port) (plugin.Reply, error) {
	return nil, &plugin.ErrorFailed{Message: "a plugin of streams only receives no request"}
}
