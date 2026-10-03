//go:build !windows

package main

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"testing"
	"time"
)

// TestDevChildProcess is the local child used to observe graceful stop at the
// process boundary; no shell, network, model or external program is involved.
func TestDevChildProcess(t *testing.T) {
	mode := os.Getenv("RELEASE_TEST_CHILD")
	if mode == "" {
		t.Skip("subprocess fixture")
	}
	ctx, stop := signal.NotifyContext(context.Background(), stopSignals()...)
	defer stop()
	if _, err := fmt.Fprintln(os.Stdout, "ready"); err != nil {
		t.Fatal(err)
	}
	if mode == "backend" {
		<-ctx.Done()
	} else {
		if _, err := io.Copy(io.Discard, os.Stdin); err != nil {
			t.Fatal(err)
		}
	}
}

func TestDevProcessesStopAndAreReaped(t *testing.T) {
	for _, mode := range []string{"backend", "manager"} {
		t.Run(mode, func(t *testing.T) {
			command := exec.Command(os.Args[0], "-test.run=^TestDevChildProcess$")
			command.Env = append(os.Environ(), "RELEASE_TEST_CHILD="+mode)
			output, err := command.StdoutPipe()
			if err != nil {
				t.Fatal(err)
			}
			input, err := command.StdinPipe()
			if err != nil {
				t.Fatal(err)
			}
			defer func() { _ = input.Close() }()
			process, err := startDevProcess(t.Context(), command)
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := process.stop(context.Background(), true, 0); err != nil {
					t.Error(err)
				}
			}()
			ready := make(chan bool, 1)
			readDone := make(chan struct{})
			go func() {
				defer close(readDone)
				reader := bufio.NewReader(output)
				line, err := reader.ReadString('\n')
				ready <- err == nil && line == "ready\n"
				_, _ = io.Copy(io.Discard, reader)
			}()
			defer func() {
				_ = output.Close()
				<-readDone
			}()
			guard, cancel := context.WithTimeout(t.Context(), 10*time.Second)
			defer cancel()
			select {
			case ok := <-ready:
				if !ok {
					t.Fatal("child exited before readiness")
				}
			case <-guard.Done():
				t.Fatal(guard.Err())
			}
			terminate := mode == "backend"
			patience := 10 * time.Second
			if !terminate {
				patience = 5 * time.Second
				if err := input.Close(); err != nil {
					t.Fatal(err)
				}
			}
			if err := process.stop(guard, terminate, patience); err != nil {
				t.Fatal(err)
			}
			if process.err != nil {
				t.Fatalf("child did not stop gracefully: %v", process.err)
			}
			if !process.command.ProcessState.Exited() {
				t.Fatal("child was not reaped")
			}
		})
	}
}
