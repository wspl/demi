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
	config, display, err := machines.ConfigFromEnv()
	if err != nil {
		fmt.Fprintln(os.Stderr, "demi-machine-manager:", err)
		if errors.Is(err, machines.ErrConfigRejected) {
			return 1
		}
		return 2
	}
	if display != "" {
		if _, err := fmt.Fprint(os.Stdout, display); err != nil {
			return 1
		}
		return 0
	}
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer stop()
	if err = machines.Run(ctx, config); err != nil {
		fmt.Fprintln(os.Stderr, "demi-machine-manager:", machines.ErrorChain(err))
		return 1
	}
	return 0
}
