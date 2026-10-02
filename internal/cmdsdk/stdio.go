package cmdsdk

import (
	"context"
	"os"

	"github.com/wspl/demi/internal/commandwire"
)

// ServeStdio takes ownership of the executable's protocol stdin and stdout.
// The executable must exit after this returns.
func ServeStdio(ctx context.Context, h Handler[commandwire.Invocation]) error {
	return Serve(ctx, &PipeConn{Reader: os.Stdin, Writer: os.Stdout}, h)
}
