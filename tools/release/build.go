package main

import (
	"context"
	"os"
	"os/exec"
)

// buildGo builds a release artifact or development program with the repository's
// Go settings. The process group owns compiler children on cancellation.
func (a *application) buildGo(ctx context.Context, target, source, destination string, release bool) error {
	args := []string{"build"}
	if release {
		args = append(args, "-trimpath", "-ldflags=-s -w")
	}
	args = append(args, "-o", destination, source)
	command := exec.CommandContext(ctx, "go", args...)
	command.Dir = a.Root
	command.Env = append(os.Environ(), "CGO_ENABLED=0")
	if target != "" {
		platform, arch := goTarget(target)
		command.Env = append(command.Env, "GOOS="+platform, "GOARCH="+arch)
	}
	command.Stdout = a.Out
	command.Stderr = a.Err
	ownBuildProcess(command)
	return command.Run()
}
