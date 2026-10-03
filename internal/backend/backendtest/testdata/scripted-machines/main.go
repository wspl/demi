// Command scripted-machines runs the backend scenarios' machine manager.
package main

import (
	"context"
	"os"
	"os/signal"
	"syscall"

	"github.com/wspl/demi/internal/backend/backendtest"
)

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	status := backendtest.ScriptedMachines(ctx, os.Args[1:], os.Stdin, os.Stdout, os.Stderr)
	stop()
	os.Exit(status)
}
