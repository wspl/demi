package process

import (
	"bytes"
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/wspl/demi/internal/cmdsdk/cmdsdktest"
)

func TestStartBusyExecutable(t *testing.T) {
	for _, bootstrap := range []bool{false, true} {
		name := "direct"
		if bootstrap {
			name = "bootstrap"
		}
		t.Run(name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "program")
			writing, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o755)
			if err != nil {
				t.Fatal(err)
			}
			// Cleanup follows the operation result; cancellation may already have closed it.
			defer func() {
				_ = writing.Close()
			}()
			if _, err := writing.WriteString("#!/bin/sh\necho started\n"); err != nil {
				t.Fatal(err)
			}
			attributes := ChildAttributes{}
			mask := uint32(0o077)
			if bootstrap {
				attributes.Umask = &mask
			}
			cmd := exec.Command(path)
			var stdout bytes.Buffer
			cmd.Stdout = &stdout
			command := Wrap(cmd, true, attributes)
			baseline := cmdsdktest.Pauses()
			done := make(chan error, 1)
			ctx := t.Context()
			go func() {
				done <- command.Start(ctx)
			}()
			// The pause counter offers no event, so the loop yields between checks.
			for cmdsdktest.Pauses() == baseline {
				select {
				case err := <-done:
					t.Fatalf("start did not wait: %v", err)
				default:
				}
				runtime.Gosched()
			}
			if err := writing.Close(); err != nil {
				t.Fatal(err)
			}
			if err := <-done; err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() {
				_ = command.Kill()
				_, _ = command.Wait(context.Background())
			})
			if err := unsuccessfulExit(command.Wait(ctx)); err != nil {
				t.Fatal(err)
			}
			if stdout.String() != "started\n" {
				t.Fatalf("stdout = %q", stdout.String())
			}
		})
	}
}
