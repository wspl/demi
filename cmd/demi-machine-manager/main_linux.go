//go:build linux

// Command demi-machine-manager runs the privileged Cloud service.
package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"github.com/wspl/demi/internal/machines"
)

func main() { os.Exit(run()) }
func run() int {
	config, err := machines.ConfigFromEnv()
	if err != nil {
		var display *machines.ConfigDisplay
		if errors.As(err, &display) {
			if _, err := fmt.Fprint(os.Stdout, display.Text); err != nil {
				return 1
			}
			return 0
		}
		fmt.Fprintln(os.Stderr, "demi-machine-manager:", err)
		var invalid *machines.InvalidConfigError
		if errors.As(err, &invalid) {
			return 2
		}
		return 1
	}
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer stop()
	if err = machines.Run(ctx, config); err != nil {
		fmt.Fprintln(os.Stderr, "demi-machine-manager:", machines.ErrorChain(err))
		return 1
	}
	return 0
}
