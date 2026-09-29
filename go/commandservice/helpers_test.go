package commandservice_test

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"io"
	"testing"
	"time"

	"github.com/wspl/demi/go/commandservice"
)

// hang is how long a test waits for an event before it decides that the code
// hangs. It guards against a hang; no test waits for it.
const hang = 30 * time.Second

func testContext(t testing.TB) context.Context {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), hang)
	t.Cleanup(cancel)
	return ctx
}

// invocation returns an invocation of operation for the conversation
// "conversation", started by agent 1.
func invocation(operation string) commandservice.Invocation {
	return commandservice.Invocation{
		Operation:    operation,
		InvocationID: operation,
		Context: commandservice.CommandContext{
			Conversation: "conversation",
			Caller:       commandservice.AgentCaller{Number: 1},
			Locale:       commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en-US"}},
		},
		Args: jsontext.Value(`{}`),
		Cwd:  "/tmp",
		Env:  map[string]string{},
	}
}

// outcome is what an invocation left: its completion and its two outputs.
type outcome struct {
	completion commandservice.Completion
	stdout     []byte
	stderr     []byte
}

// run drives an invocation to its completion with input as its input.
func run(t testing.TB, client *commandservice.Client, invocation commandservice.Invocation, input []byte) outcome {
	t.Helper()
	ctx := testContext(t)
	stream, err := client.Invoke(ctx, invocation)
	if err != nil {
		t.Fatalf("invoke %s: %v", invocation.Operation, err)
	}
	var stdout, stderr bytes.Buffer
	completion, err := commandservice.Exchange(ctx, stream, bytes.NewReader(input), &stdout, &stderr)
	if err != nil {
		t.Fatalf("%s: %v", invocation.Operation, err)
	}
	return outcome{completion: completion, stdout: stdout.Bytes(), stderr: stderr.Bytes()}
}

// drive runs an invocation with no input to its completion, and discards its
// output. It is safe to call from any goroutine.
func drive(ctx context.Context, client *commandservice.Client, invocation commandservice.Invocation) (commandservice.Completion, error) {
	stream, err := client.Invoke(ctx, invocation)
	if err != nil {
		return commandservice.Completion{}, err
	}
	return commandservice.Exchange(ctx, stream, bytes.NewReader(nil), io.Discard, io.Discard)
}

// converse sends a request to the conversation endpoint and returns what it
// printed and how it completed. It is safe to call from any goroutine.
func converse(ctx context.Context, client *commandservice.Client, request commandservice.ConversationRequest) (string, commandservice.Completion, error) {
	stream, err := client.Conversation(ctx, request)
	if err != nil {
		return "", commandservice.Completion{}, err
	}
	var stdout, stderr bytes.Buffer
	completion, err := commandservice.Exchange(ctx, stream, bytes.NewReader(nil), &stdout, &stderr)
	return stdout.String(), completion, err
}

// conversation is converse that fails the test on an error.
func conversation(t testing.TB, client *commandservice.Client, request commandservice.ConversationRequest) (string, commandservice.Completion) {
	t.Helper()
	stdout, completion, err := converse(testContext(t), client, request)
	if err != nil {
		t.Fatalf("conversation %+v: %v", request, err)
	}
	return stdout, completion
}

func release(conversation string) commandservice.ConversationRequest {
	return commandservice.ReleaseRequest{Conversation: conversation}
}

func status() commandservice.ConversationRequest {
	return commandservice.StatusRequest{}
}
