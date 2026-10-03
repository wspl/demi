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
			writing, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0755)
			if err != nil {
				t.Fatal(err)
			}
			defer func() { _ = writing.Close() }() // Cleanup follows the operation result; cancellation may already have closed it.
			if _, err := writing.WriteString("#!/bin/sh\necho started\n"); err != nil {
				t.Fatal(err)
			}
			attributes := ChildAttributes{}
			mask := uint32(0077)
			if bootstrap {
				attributes.Umask = &mask
			}
			cmd := exec.Command(path)
			var stdout bytes.Buffer
			cmd.Stdout = &stdout
			command := Wrap(cmd, true, attributes)
			baseline := cmdsdktest.Pauses()
			done := make(chan error, 1)
			ctx := childContext(t)
			go func() { done <- command.Start(ctx) }()
			for cmdsdktest.Pauses() == baseline {
				select {
				case err := <-done:
					t.Fatalf("start did not wait: %v", err)
				default:
				}
				if err := ctx.Err(); err != nil {
					t.Fatal(err)
				}
				runtime.Gosched()
			}
			if err := writing.Close(); err != nil {
				t.Fatal(err)
			}
			if err := <-done; err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { _ = command.Kill(); command.Wait(context.Background()) })
			requireSuccess(t, command.Wait(ctx))
			if stdout.String() != "started\n" {
				t.Fatalf("stdout = %q", stdout.String())
			}
		})
	}
}
