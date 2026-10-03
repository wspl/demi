package backendtest

//revive:disable:unused-parameter

import (
	"context"
	"io"

	"github.com/wspl/demi/internal/plugin"
)

// Pattern makes a backend transfer fixture of length bytes varied by seed,
// repeating only every 64,256 bytes so reads from shifted offsets are visible.
func Pattern(length int, seed byte) []byte { panic("not written: b-backend") }

// StreamsPlugin declares user streams bound to a scenario's native fixture.
// It serves the same manifest through the normal plugin registration boundary.
func StreamsPlugin(streams []plugin.Stream) plugin.Factory { panic("not written: b-backend") }

// ScriptedMachines runs the scenario machine manager as a program. It accepts
// --artifacts <directory>, prints the socket path as its first output line,
// and serves until input ends or ctx is canceled. It then stops and joins all
// runners. It owns and closes input to unblock its reader during cancellation.
// args excludes argv[0]; the caller handles process signals through ctx.
func ScriptedMachines(ctx context.Context, args []string, input io.ReadCloser, output, diagnostics io.Writer) int {
	panic("not written: b-backend")
}
