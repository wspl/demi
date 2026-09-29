// Command demi-backend is the Demi product server: it reads its
// configuration, serves until SIGINT or SIGTERM, then shuts down in order
// (backend.md § Startup and shutdown).
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"log/slog"
	"os"
	"os/signal"
	"syscall"

	"github.com/wspl/demi/go/backend/edge"
	"github.com/wspl/demi/go/internal/envflag"
)

func main() {
	os.Exit(run())
}

func run() int {
	// An unusable value stops here, naming its variable.
	settings, err := parseSettings(os.Args[1:], os.LookupEnv, os.Stdout)
	if errors.Is(err, flag.ErrHelp) {
		return 0
	}
	if err != nil {
		fmt.Fprintf(os.Stderr, "demi-backend: %v\n", err)
		if envflag.IsUsage(err) {
			return 2
		}
		return 1
	}
	text := slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: levelTrace, ReplaceAttr: levelNames})
	slog.SetDefault(slog.New(newFiltered(settings.log, text)))
	config, err := settings.backendConfig()
	if err != nil {
		return failed(err)
	}
	// The native releases DEMI_NATIVE_CONFIG names are published before the
	// backend accepts requests once the runners' packages are ported (G7g).
	control, err := edge.ControlFromEnvironment(&config)
	if err != nil {
		return failed(err)
	}
	stopped, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	server, err := edge.Start(context.Background(), config)
	if err != nil {
		return failed(err)
	}
	slog.Info("demi-backend is listening", "address", server.Addr().String(), "data", config.DataDir, "mode", string(config.Mode))
	if control != nil {
		// A test build serves its control socket meanwhile; one that cannot
		// bind it stops the backend.
		if err := control.Serve(server); err != nil {
			if closeErr := server.Close(context.Background()); closeErr != nil {
				fmt.Fprintf(os.Stderr, "demi-backend: %v\n", closeErr)
			}
			return failed(err)
		}
	}
	<-stopped.Done()
	if control != nil {
		control.Stop()
	}
	closed := server.Close(context.Background())
	if control != nil {
		// What the control's connections hold stays held through the
		// shutdown, so that a test can hold a flow at the moment the
		// backend closes.
		control.End()
	}
	if closed != nil {
		return failed(closed)
	}
	return 0
}

func failed(err error) int {
	fmt.Fprintf(os.Stderr, "demi-backend: %v\n", err)
	return 1
}

// levelNames writes tracing's trace as TRACE rather than DEBUG-4.
func levelNames(groups []string, attr slog.Attr) slog.Attr {
	if len(groups) == 0 && attr.Key == slog.LevelKey && attr.Value.Any() == levelTrace {
		return slog.String(slog.LevelKey, "TRACE")
	}
	return attr
}
