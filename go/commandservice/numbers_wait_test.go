package commandservice

import (
	"bytes"
	"context"
	"io"
	"net"
	"testing"
	"time"
)

type waitingNumbersHandler struct{ source chan *Numbers }

func (h waitingNumbersHandler) Operations() []string { return []string{"number"} }

func (h waitingNumbersHandler) Invoke(call *Call) (Completion, error) {
	h.source <- call.Numbers
	first, err := call.Numbers.Draw(call.Context(), "c1", Tab, 1)
	if err != nil {
		return Completion{}, err
	}
	if first != 9 {
		return Completion{}, &InvalidError{Field: "first", Rule: "expected reserved number 9"}
	}
	_, err = call.Stdout.Write([]byte("reserved"))
	return Completion{}, err
}

// Budget ten seconds. Observing the queued draw is solely an event barrier:
// the assertions concern Draw completing through the public client and numbers API.
func TestDrawActuallyWaitsBeforeNumbersOpen(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	a, b := net.Pipe()
	done := make(chan error, 1)
	t.Cleanup(func() {
		cancel()
		// Both ends can already be closed after shutdown.
		_ = a.Close()
		_ = b.Close()
		if err := <-done; err != nil {
			t.Error(err)
		}
	})
	handler := waitingNumbersHandler{source: make(chan *Numbers, 1)}
	go func() { done <- Serve(ctx, a, handler) }()
	client, err := Connect(ctx, b)
	if err != nil {
		t.Fatal(err)
	}
	invocation := Invocation{
		Operation: "number", InvocationID: "test", Args: []byte(`{}`), Cwd: "/tmp", Env: map[string]string{},
		Context: CommandContext{
			Conversation: "c1",
			Caller:       CommandCaller{Kind: "user"},
			Locale:       CommandLocale{TimeZone: "UTC", Languages: []string{"en"}},
		},
	}
	stream, err := client.Invoke(ctx, invocation)
	if err != nil {
		t.Fatal(err)
	}
	var source *Numbers
	select {
	case source = <-handler.source:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	select {
	case request := <-source.draws:
		source.draws <- request
	case <-ctx.Done():
		t.Fatal("Draw did not queue before the stream opened")
	}
	numbers, err := client.Numbers(ctx)
	if err != nil {
		t.Fatal(err)
	}
	answered := make(chan error, 1)
	go func() {
		answered <- numbers.Answer(ctx, func(context.Context, NumbersRequest) (uint64, error) { return 9, nil })
	}()
	var output bytes.Buffer
	if _, err = Exchange(ctx, stream, bytes.NewReader(nil), &output, io.Discard); err != nil {
		t.Fatal(err)
	}
	if output.String() != "reserved" {
		t.Fatal(output.String())
	}
	if err = client.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	if err = <-answered; err != nil {
		t.Fatal(err)
	}
}
