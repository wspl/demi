package claudecode

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"io"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandpackage/claudecode/claudecodeproto"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/commandsdk/commandsdktest"
	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

type programSuite struct{ m *testing.M }

func (s programSuite) Run() int { return programtest.Run(s.m) }
func TestMain(m *testing.M)     { goleak.VerifyTestMain(programSuite{m}) }

type releaseInput struct{ remaining []byte }

func (i *releaseInput) Next(ctx context.Context) ([]byte, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if len(i.remaining) == 0 {
		return nil, io.EOF
	}
	n := min(len(i.remaining), 64*1024)
	chunk := i.remaining[:n]
	i.remaining = i.remaining[n:]
	return chunk, nil
}

type written struct{ stdout, stderr bytes.Buffer }

func (w *written) Stdout(_ context.Context, b []byte) error {
	_, err := w.stdout.Write(b)
	return err
}

func (w *written) Stderr(_ context.Context, b []byte) error {
	_, err := w.stderr.Write(b)
	return err
}

func invokeProgram(
	t *testing.T,
	client *commandsdk.Client,
	operation string,
	input []byte,
) ([]byte, commandproto.Completion) {
	t.Helper()
	writer, output, err := client.Invoke(t.Context(), commandproto.Invocation{
		Operation:    operation,
		InvocationID: "invocation",
		Context: commandproto.Context{
			Conversation: "conversation",
			Caller:       &commandproto.AgentCaller{Number: 1},
			Locale:       commandproto.CommandLocale{TimeZone: "UTC", Languages: []commandproto.LanguageTag{"en-US"}},
		},
		Args: []byte(`{}`),
		Cwd:  "/",
		Env:  map[string]string{},
	})
	if err != nil {
		t.Fatal(err)
	}
	var outputBytes written
	completion, err := (commandsdk.Exchange{Input: writer, Output: output}).Run(
		t.Context(),
		&releaseInput{input},
		&outputBytes,
	)
	if err != nil {
		t.Fatal(err)
	}
	body := outputBytes.stdout.Bytes()
	if outputBytes.stderr.Len() != 0 || !bytes.HasSuffix(body, []byte{'\n'}) {
		t.Fatalf("stdout=%q stderr=%q", body, outputBytes.stderr.String())
	}
	return bytes.TrimSuffix(body, []byte{'\n'}), completion
}

// This starts the actual command executable to protect stdio wiring and the
// complete invocation lifecycle; its one incremental build dominates the cost.
func TestServiceAnswersOneDocumentPerInvocation(t *testing.T) {
	path, err := programtest.Path(t.Context(), "demi-claude-code")
	if err != nil {
		t.Fatal(err)
	}
	again, err := programtest.Path(t.Context(), "demi-claude-code")
	if err != nil || again != path {
		t.Fatalf("cached program = %q, %v", again, err)
	}
	process, err := commandsdktest.Start(t.Context(), t, path, []string{"--command-service"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	stream, err := process.Client.Artifacts(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	answered := make(chan error, 1)
	go func() {
		answered <- stream.AnswerArtifacts(ctx, func(
			_ context.Context,
			request commandproto.ArtifactRequest,
		) (commandproto.ArtifactAnswer, error) {
			path := "/cache/claude<&\u2028>"
			if request.Install != nil {
				return commandproto.ArtifactAnswer{Path: &path}, nil
			}
			if request.Installed == nil || request.Installed.Name != "Claude Code" {
				return commandproto.ArtifactAnswer{}, fmt.Errorf("unexpected request: %+v", request)
			}
			// The runner lists the newest install first; status keeps that order.
			installed := []commandproto.InstalledArtifact{{
				Version: "2.1.278",
				Path:    path,
				SHA256:  fmt.Sprintf("%x", sha256.Sum256([]byte("claude"))),
			}, {
				Version: "2.1.10",
				Path:    "/cache/2.1.10",
				SHA256:  fmt.Sprintf("%x", sha256.Sum256([]byte("2.1.10"))),
			}}
			return commandproto.ArtifactAnswer{Installed: &installed}, nil
		})
	}()
	t.Cleanup(func() {
		cancel()
		if err := <-answered; err != nil && !errors.Is(err, context.Canceled) {
			t.Errorf("artifact stream: %v", err)
		}
	})
	info, err := process.Client.Info(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if got, want := info.Operations, []string{
		"claude-code.ensure",
		"claude-code.status",
	}; !reflect.DeepEqual(
		got,
		want,
	) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
	body, completion := invokeProgram(
		t,
		process.Client,
		"claude-code.ensure",
		releaseRecord("2.1.278", currentPlatform()),
	)
	reply, err := claudecodeproto.DecodeEnsureReply(body)
	if err != nil {
		t.Fatal(err)
	}
	if got, want := reply, claudecodeproto.EnsureReply(
		&claudecodeproto.Ensured{
			Installed: claudecodeproto.Installed{Version: "2.1.278", Path: "/cache/claude<&\u2028>"},
		},
	); !reflect.DeepEqual(
		got,
		want,
	) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
	if completion.ExitCode != 0 || completion.Error != nil {
		t.Fatalf("completion: %+v", completion)
	}
	if bytes.Contains(body, []byte(`\u003c`)) || bytes.Contains(body, []byte(`\u2028`)) {
		t.Fatalf("escaped reply: %s", body)
	}
	body, completion = invokeProgram(t, process.Client, "claude-code.status", nil)
	statusReply, err := claudecodeproto.DecodeStatusReply(body)
	if err != nil {
		t.Fatal(err)
	}
	want := claudecodeproto.StatusReply(
		&claudecodeproto.StatusDone{
			Status: claudecodeproto.Status{
				Platform: currentPlatform(),
				Installed: []claudecodeproto.Installed{
					{Version: "2.1.278", Path: "/cache/claude<&\u2028>"},
					{Version: "2.1.10", Path: "/cache/2.1.10"},
				},
			},
		},
	)
	if got, want := statusReply, want; !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
	if completion.ExitCode != 0 {
		t.Fatalf("completion: %+v", completion)
	}
	for _, input := range [][]byte{
		[]byte(strings.Replace(
			string(releaseRecord("2.1.279", currentPlatform())),
			"https://downloads.claude.ai/claude",
			"http://127.0.0.1:9/claude",
			1,
		)),
		[]byte(`{}`), bytes.Repeat([]byte(" "), 64*1024+1),
	} {
		body, completion = invokeProgram(t, process.Client, "claude-code.ensure", input)
		reply, err := claudecodeproto.DecodeEnsureReply(body)
		if err != nil {
			t.Fatal(err)
		}
		failure, ok := reply.(*claudecodeproto.Failed)
		if !ok || failure.Code != claudecodeproto.InvalidRelease || completion.ExitCode != 1 ||
			completion.Error == nil {
			t.Fatalf("reply=%+v completion=%+v", reply, completion)
		}
		if completion.Error.Code != string(failure.Code) || completion.Error.Message != failure.Message {
			t.Fatalf("completion differs from reply: %+v %+v", completion.Error, failure)
		}
	}
	if err := process.Shutdown(t.Context()); err != nil {
		t.Fatal(err)
	}
}
