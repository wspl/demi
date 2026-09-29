//go:build unix

package interp_test

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

// The allocation policy is a Host boundary. These scenarios ensure that the
// interpreter consults it for each allocation formerly bypassing the Host.
// They run builtin-only jobs and use no timed waits.
func TestInternalAllocationsUseHostPolicy(t *testing.T) {
	for _, scenario := range []struct {
		name, script, want string
		calls              int64
	}{
		{"directory", "printf '%s' *", "entry", 1},
		{"read wake pipe", "read line; printf '%s' \"$line\"", "payload", 1},
		{"substitution FIFO", "read line < <(printf 'payload\\n'); printf '%s' \"$line\"", "payload", 3},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			dir := t.TempDir()
			if err := os.WriteFile(filepath.Join(dir, "entry"), nil, 0600); err != nil {
				t.Fatal(err)
			}
			input, err := os.CreateTemp(t.TempDir(), "input")
			if err != nil {
				t.Fatal(err)
			}
			defer input.Close()
			if _, err := input.WriteString("payload\n"); err != nil {
				t.Fatal(err)
			}
			if _, err := input.Seek(0, 0); err != nil {
				t.Fatal(err)
			}
			var out bytes.Buffer
			var calls atomic.Int64
			runner, err := interp.New(interp.Dir(dir), interp.StdIO(input, &out, &out), interp.RetryHandler(func(ctx context.Context, attempt func() error) error {
				calls.Add(1)
				return attempt()
			}))
			if err != nil {
				t.Fatal(err)
			}
			file, err := syntax.NewParser().Parse(strings.NewReader(scenario.script), "")
			if err != nil {
				t.Fatal(err)
			}
			if err := runner.Run(t.Context(), file); err != nil {
				t.Fatal(err)
			}
			runner.WaitBackground()
			if out.String() != scenario.want || calls.Load() != scenario.calls {
				t.Fatalf("output=%q policy calls=%d", out.String(), calls.Load())
			}
		})
	}
}

func TestFIFOCancellationUsesHostPolicy(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	entered := make(chan struct{})
	var calls atomic.Int64
	runner, err := interp.New(interp.RetryHandler(func(ctx context.Context, attempt func() error) error {
		if calls.Add(1) == 1 {
			close(entered)
		}
		return attempt()
	}))
	if err != nil {
		t.Fatal(err)
	}
	file, err := syntax.NewParser().Parse(strings.NewReader(": <(printf unused)"), "")
	if err != nil {
		t.Fatal(err)
	}
	if err := runner.Run(ctx, file); err != nil {
		t.Fatal(err)
	}
	<-entered
	cancel()
	runner.WaitBackground()
	if calls.Load() != 2 {
		t.Fatalf("FIFO open and cancellation rendezvous: policy called %d times", calls.Load())
	}
}
