package cmdpkgstest_test

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/cmdsdk/cmdsdktest"
	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	code := programtest.Run(m)
	if err := goleak.Find(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		code = 1
	}
	os.Exit(code)
}

// This exercises the built fixture program through its public HTTP/2 boundary;
// the one cold build is shared by programtest and can take more than a second.
func TestFixtureIOContextAndNumbers(t *testing.T) {
	path, err := programtest.Path(t.Context(), "demi-native-fixture")
	if err != nil {
		t.Fatal(err)
	}
	service, err := cmdsdktest.Start(t.Context(), t, path, []string{cmdsdk.CommandService}, nil)
	if err != nil {
		t.Fatal(err)
	}
	stream, err := service.Client.Numbers(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan error, 1)
	go func() {
		done <- stream.AnswerNumbers(ctx, func(_ context.Context, q cmdproto.NumbersRequest) (uint64, error) {
			if q.Count != 2 || q.Conversation != "c" {
				return 0, fmt.Errorf("unexpected draw: %+v", q)
			}
			return 42, nil
		})
	}()
	defer func() {
		cancel()
		<-done
	}()
	for _, tc := range []struct {
		operation, args, input, stdout, stderr string
		code                                   uint8
	}{
		{"where", `{"label":"<>&"}`, "", `"label":"<>&"`, "", 0},
		{"where", `{}`, "", `"label":null`, "", 0},
		{"number", `{"count":2}`, "", `{"first":42}`, "", 0},
		{"result", `{}`, "", "command output", "command diagnostic", 17},
		{"first", `{}`, "input chunk", "input chunk", "", 0},
	} {
		t.Run(tc.operation, func(t *testing.T) {
			request := cmdproto.Invocation{
				Operation:    tc.operation,
				InvocationID: tc.operation,
				Args:         []byte(tc.args),
				Cwd:          os.TempDir(),
				Env:          map[string]string{"PROBE": "value"},
				Context: cmdproto.Context{
					Conversation: "c",
					Caller:       &cmdproto.AgentCaller{Number: 1},
					Locale: cmdproto.CommandLocale{
						TimeZone:  "UTC",
						Languages: []cmdproto.LanguageTag{"en-US"},
					},
				},
			}
			input, output, err := service.Client.Invoke(t.Context(), request)
			if err != nil {
				t.Fatal(err)
			}
			defer input.Cancel()
			sent := false
			var stdout, stderr strings.Builder
			var completion *cmdproto.Completion
			for {
				record, err := output.Next(t.Context())
				if errors.Is(err, io.EOF) {
					break
				}
				if err != nil {
					t.Fatal(err)
				}
				switch v := record.(type) {
				case cmdproto.Stdout:
					stdout.Write(v)
				case cmdproto.Stderr:
					stderr.Write(v)
				case cmdproto.Completed:
					completion = &v.Completion
				case cmdproto.InputPull:
					if !sent && tc.input != "" {
						err = input.Write(t.Context(), []byte(tc.input))
						sent = true
					} else {
						err = input.End()
					}
					if err != nil {
						t.Fatal(err)
					}
				}
			}
			if completion == nil || completion.ExitCode != tc.code || !strings.Contains(stdout.String(), tc.stdout) ||
				stderr.String() != tc.stderr {
				t.Fatalf("completion=%+v stdout=%q stderr=%q", completion, stdout.String(), stderr.String())
			}
		})
	}
	cancel()
	// Shutdown still completes with the numbers stream cancelled.
	if err := service.Shutdown(t.Context()); err != nil {
		t.Fatal(err)
	}
}
