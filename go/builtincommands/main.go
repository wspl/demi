package builtincommands

import (
	"context"
	"fmt"
	"log/slog"
	"os"

	"github.com/wspl/demi/go/commandservice"
)

// SweepOrphans removes what a service that ended without retiring its browsers
// left behind: the profiles of browsers no service owns
// (docs/browser/browser.md § Native driver). The conversation browser arrives
// with its profiles and its process discovery; until then there is nothing that
// could be left, and the sweep finds nothing to do.
func SweepOrphans(ctx context.Context) {}

// Main serves the package over standard input and output, as a runner starts it,
// and exits the process: with status 0 when the service ended, else it says why
// on standard error and exits with status 1. Any other invocation of the program
// than --command-service exits with status 2.
func Main() {
	if len(os.Args) < 2 || os.Args[1] != "--command-service" {
		fmt.Fprintln(os.Stderr, "Usage: demi-commands --command-service")
		os.Exit(2)
	}
	// Diagnostics go to standard error, which the runner drains into the Host's
	// log line by line, and which the log adds its own time to.
	slog.SetDefault(slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{
		Level: slog.LevelInfo,
		ReplaceAttr: func(groups []string, attr slog.Attr) slog.Attr {
			if len(groups) == 0 && attr.Key == slog.TimeKey {
				return slog.Attr{}
			}
			return attr
		},
	})))
	ctx := context.Background()
	// The sweep runs beside serving, and the program ends when both have.
	swept := make(chan struct{})
	go func() {
		defer close(swept)
		SweepOrphans(ctx)
	}()
	err := commandservice.ServeStdio(ctx, New())
	<-swept
	if err != nil {
		fmt.Fprintf(os.Stderr, "demi-commands: %v\n", err)
		os.Exit(1)
	}
	// The executable owns the process's lifetime: HTTP/2 has drained.
	os.Exit(0)
}
