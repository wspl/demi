package builtincommands

import (
	"context"
	"encoding/json/jsontext"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// A mutation that waits for the gate gives up its place when its invocation is
// cancelled, so a service that is asked to end is not held up by it (a handler
// that does not stop soon after its call ends is a fault of the service).
func TestAMutationThatWaitsForTheGateStopsWaitingWhenItsInvocationIsCancelled(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		handler := New()
		server := servicetest.Start(t, handler)
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		// Another mutation holds the gate, and does not let go until the test ends.
		held, err := handler.mutations.Acquire(ctx)
		if err != nil {
			t.Fatal(err)
		}
		defer held.Release()
		stream, err := server.Client.Invoke(ctx, commandservice.Invocation{
			Operation: "file.create", InvocationID: "waits",
			Context: commandservice.CommandContext{Conversation: "c", Caller: commandservice.UserCaller{}, Locale: commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en"}}},
			Args:    jsontext.Value(`{"path":"a.txt","content":"a"}`), Cwd: t.TempDir(), Env: map[string]string{},
		})
		if err != nil {
			t.Fatal(err)
		}
		// Once everything that can run has, the invocation waits for the gate.
		synctest.Wait()
		stream.Cancel()
		synctest.Wait()
		if err := server.Client.Shutdown(ctx); err != nil {
			t.Fatal(err)
		}
		if err := server.Wait(ctx); err != nil {
			t.Errorf("the service ended with %v; the mutation kept waiting for the gate", err)
		}

	})
}
