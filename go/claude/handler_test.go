package claude_test

import (
	"bytes"
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/wspl/demi/go/claude"
	"github.com/wspl/demi/go/claudeproto"
	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// invocation returns an invocation of operation.
func invocation(operation string) commandservice.Invocation {
	return commandservice.Invocation{
		Operation:    operation,
		InvocationID: "invocation",
		Context: commandservice.CommandContext{
			Conversation: "conversation",
			Caller:       commandservice.AgentCaller{Number: 1},
			Locale:       commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en-US"}},
		},
		Args: []byte(`{}`),
		Cwd:  "/",
		Env:  map[string]string{},
	}
}

// invoke invokes operation with input as its standard input, and returns the
// document the service wrote and its completion.
func invoke(ctx context.Context, t *testing.T, client *commandservice.Client, operation string, input []byte) (string, commandservice.Completion) {
	t.Helper()
	stream, err := client.Invoke(ctx, invocation(operation))
	if err != nil {
		t.Fatal(err)
	}
	var stdout, stderr bytes.Buffer
	completion, err := commandservice.Exchange(ctx, stream, bytes.NewReader(input), &stdout, &stderr)
	if err != nil {
		t.Fatal(err)
	}
	if stderr.Len() != 0 {
		t.Errorf("%s wrote %q to standard error", operation, stderr.Bytes())
	}
	document, ok := strings.CutSuffix(stdout.String(), "\n")
	if !ok {
		t.Fatalf("%s wrote %q, which is not a line", operation, stdout.Bytes())
	}
	return document, completion
}

func TestTheServiceAnswersOneDocumentForEachInvocation(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	service := servicetest.Start(t, claude.NewHandler(m.installer()))
	ctx := t.Context()
	info, err := service.Client.Info(ctx)
	if err != nil || len(info.Operations) != 2 {
		t.Fatalf("info = %+v, %v", info, err)
	}
	path := m.home + "/2.1.278/" + binary()

	document, completion := invoke(ctx, t, service.Client, "claude.ensure", served(t, server, "2.1.278", len(body), sha256Hex(body)))
	if want := `{"ok":true,"version":"2.1.278","path":"` + path + `"}`; document != want || completion.ExitCode != 0 || completion.Error != nil {
		t.Errorf("ensure: %s, %+v; want %s", document, completion, want)
	}
	document, completion = invoke(ctx, t, service.Client, "claude.status", nil)
	want := `{"ok":true,"platform":"` + platform(t) + `","installed":[{"version":"2.1.278","path":"` + path + `"}]}`
	if document != want || completion.ExitCode != 0 {
		t.Errorf("status: %s, %+v; want %s", document, completion, want)
	}
}

func TestAFailureIsADocumentAndACompletionToo(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	service := servicetest.Start(t, claude.NewHandler(m.installer()))
	ctx := t.Context()
	document, completion := invoke(ctx, t, service.Client, "claude.ensure", served(t, server, "2.1.278", len(body), sha256Hex([]byte("another executable"))))
	if completion.ExitCode != 1 || completion.Error == nil || completion.Error.Code != "verification_failed" {
		t.Fatalf("completion %+v", completion)
	}
	reply, err := claudeproto.Decode[claudeproto.Reply]([]byte(document))
	failed, ok := reply.(claudeproto.Failed)
	if err != nil || !ok || failed.Code != claudeproto.VerificationFailed || failed.Message != completion.Error.Message {
		t.Errorf("the document %s decodes as %#v, %v", document, reply, err)
	}
	// A record that is none, one that is not HTTPS, and one that is too long.
	for name, input := range map[string][]byte{
		"a record with no member": []byte(`{}`),
		"plain HTTP":              record(t, "2.1.279", platform(t), "http://downloads.claude.ai/claude", len(body), sha256Hex(body)),
		"an input over 64 KiB":    bytes.Repeat([]byte(" "), 64*1024+1),
	} {
		document, completion := invoke(ctx, t, service.Client, "claude.ensure", input)
		want := `{"ok":false,"code":"invalid_release","message":` + quoted(completion.Error) + `}`
		if completion.ExitCode != 1 || completion.Error == nil || completion.Error.Code != "invalid_release" || document != want {
			t.Errorf("%s: %s, %+v", name, document, completion)
		}
	}
	document, _ = invoke(ctx, t, service.Client, "claude.ensure", bytes.Repeat([]byte(" "), 64*1024+1))
	if !strings.Contains(document, "input exceeds 65536 bytes") {
		t.Errorf("an input over 64 KiB: %s", document)
	}
}

func TestAnInputOfExactly64KiBIsRead(t *testing.T) {
	m := newMachine(t)
	server := downloads(t, nil)
	service := servicetest.Start(t, claude.NewHandler(m.installer()))
	input := served(t, server, "2.1.278", len(body), sha256Hex(body))
	input = append(input, bytes.Repeat([]byte(" "), 64*1024-len(input))...)
	document, completion := invoke(t.Context(), t, service.Client, "claude.ensure", input)
	if completion.ExitCode != 0 || !strings.HasPrefix(document, `{"ok":true`) {
		t.Errorf("an input of 65536 bytes: %s, %+v", document, completion)
	}
}

func quoted(failure *commandservice.CommandError) string {
	if failure == nil {
		return ""
	}
	data, err := claudeproto.Encode(claudeproto.Failed{Code: claudeproto.InvalidRelease, Message: failure.Message})
	if err != nil {
		return ""
	}
	// The message is the last member of the document.
	_, message, _ := strings.Cut(string(data), `"message":`)
	return strings.TrimSuffix(message, "}")
}

func TestACancelledInvocationWritesNoDocument(t *testing.T) {
	m := newMachine(t)
	hold := make(chan struct{})
	defer close(hold)
	server := downloads(t, hold)
	service := servicetest.Start(t, claude.NewHandler(m.installer()))
	ctx := t.Context()
	stream, err := service.Client.Invoke(ctx, invocation("claude.ensure"))
	if err != nil {
		t.Fatal(err)
	}
	completed := make(chan error, 1)
	go func() {
		var stdout bytes.Buffer
		_, err := commandservice.Exchange(ctx, stream, bytes.NewReader(served(t, server, "2.1.278", len(body), sha256Hex(body))), &stdout, &stdout)
		if err == nil {
			err = errors.New("the invocation completed: " + stdout.String())
		}
		completed <- err
	}()
	<-server.Arrived()
	stream.Cancel()
	if err := <-completed; !errors.Is(err, context.Canceled) {
		t.Errorf("a cancelled invocation ended with %v", err)
	}
	// The service goes on.
	if document, completion := invoke(ctx, t, service.Client, "claude.status", nil); completion.ExitCode != 0 || !strings.HasPrefix(document, `{"ok":true`) {
		t.Errorf("status after a cancelled invocation: %s, %+v", document, completion)
	}
	// Serve returns once its calls have stopped, and the download's stage is gone
	// with its call.
	service.Stop()
	if err := service.Wait(ctx); err != nil {
		t.Errorf("the service ended with %v", err)
	}
	if got := m.homeDirectories(t); len(got) != 0 {
		t.Errorf("the user's root holds %v", got)
	}
}
