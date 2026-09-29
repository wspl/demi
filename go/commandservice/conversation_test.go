package commandservice_test

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

func TestAServiceWithoutConversationStateAnswersStatusAndRelease(t *testing.T) {
	server := servicetest.Start(t, operations{"noop": short})
	for _, test := range []struct {
		request commandservice.ConversationRequest
		answer  string
	}{
		{status(), `{"conversations":[]}`},
		{release("unknown"), `{}`},
		{release("unknown"), `{}`},
	} {
		answer, completion := conversation(t, server.Client, test.request)
		if answer != test.answer || completion.ExitCode != 0 || completion.Error != nil {
			t.Errorf("%+v answered %s %+v, want %s and exit code 0", test.request, answer, completion, test.answer)
		}
	}
}

// lifecycle is a handler with conversation state: a release of "wait" waits
// for its cancellation, one of "resist" waits and then fails, one of "abandon"
// waits and then exits with status 3, one of "exit" exits with status 3 at
// once, one of "panic" panics, and any other release fails.
type lifecycle struct {
	started   signal
	cancelled signal
	closed    signal
	order     events
}

func (l *lifecycle) Operations() []string { return []string{"noop"} }

func (l *lifecycle) Invoke(*commandservice.Call) (commandservice.Completion, error) {
	return commandservice.Completion{}, nil
}

func (l *lifecycle) Conversation(call *commandservice.ConversationCall) (commandservice.Completion, error) {
	l.started.send()
	release := call.Request.Operation == commandservice.OperationRelease
	switch {
	case release && call.Request.Conversation == "wait":
		<-call.Context().Done()
		l.order.add("release stopped")
		l.cancelled.send()
		return commandservice.Completion{}, call.Context().Err()
	case release && call.Request.Conversation == "resist":
		<-call.Context().Done()
		return commandservice.Completion{}, errors.New("cleanup was cut short")
	case release && call.Request.Conversation == "abandon":
		<-call.Context().Done()
		return commandservice.Completion{ExitCode: 3}, nil
	case release && call.Request.Conversation == "exit":
		return commandservice.Completion{ExitCode: 3}, nil
	case release && call.Request.Conversation == "panic":
		panic("cleanup broke")
	}
	return commandservice.Completion{}, errors.New("profile cleanup failed")
}

func (l *lifecycle) Close(context.Context) error {
	l.order.add("close")
	l.closed.send()
	return nil
}

func TestConversationCancellationJoinsTheHandlerAndAFailedCleanupRetiresTheService(t *testing.T) {
	handler := &lifecycle{started: newSignal(), cancelled: newSignal(), closed: newSignal()}
	server := servicetest.Start(t, handler)
	client := server.Client

	waiting, err := client.Conversation(testContext(t), release("wait"))
	if err != nil {
		t.Fatal(err)
	}
	handler.started.receive(t)
	waiting.Cancel()
	handler.cancelled.receive(t)
	// A release cancelled while it waited is no failed cleanup: the service
	// answers on.
	if _, err := client.Info(testContext(t)); err != nil {
		t.Fatalf("the service after a cancelled release: %v", err)
	}

	// The failure closes the service; the final record may race the teardown
	// of the connection, but it never reports a successful completion.
	if _, completion, err := converse(testContext(t), client, release("fail")); err == nil && completion.ExitCode == 0 {
		t.Errorf("the release of a conversation whose cleanup fails completed with %+v", completion)
	}
	err = server.Wait(testContext(t))
	want := "conversation cleanup failed; retire the service process: release exited with status 1: command_failed: profile cleanup failed"
	if !errors.Is(err, commandservice.ErrConversationCleanup) || err == nil || err.Error() != want {
		t.Errorf("Serve returned %v, want %q", err, want)
	}
	handler.closed.receive(t)
	if got := handler.order.all(); len(got) != 2 || got[0] != "release stopped" || got[1] != "close" {
		t.Errorf("order = %v, want the cancelled release to stop before the handler closes", got)
	}
}

func TestAReleaseThatEndsBadlyRetiresTheService(t *testing.T) {
	silenceLog(t)
	for name, test := range map[string]struct {
		conversation string
		cancel       bool
		cause        string
	}{
		"fails when cancelled":         {"resist", true, "cleanup was cut short"},
		"exits nonzero when cancelled": {"abandon", true, "status 3"},
		"exits nonzero":                {"exit", false, "status 3"},
		"panics":                       {"panic", false, "cleanup broke"},
	} {
		handler := &lifecycle{started: newSignal(), cancelled: newSignal(), closed: newSignal()}
		server := servicetest.Start(t, handler)
		stream, err := server.Client.Conversation(testContext(t), release(test.conversation))
		if test.cancel {
			if err != nil {
				t.Fatalf("%s: %v", name, err)
			}
			handler.started.receive(t)
			stream.Cancel()
		}
		// Without the cancellation the release ends at once, and the service
		// retires itself while its caller may still be sending the request: only
		// how Serve returns is asserted.
		err = server.Wait(testContext(t))
		if !errors.Is(err, commandservice.ErrConversationCleanup) || !strings.Contains(err.Error(), test.cause) {
			t.Errorf("%s: Serve returned %v, want a conversation cleanup failure caused by %q", name, err, test.cause)
		}
	}
}
