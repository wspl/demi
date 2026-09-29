package backendtest_test

import (
	"net/http"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

// Cost: one backend and a real runner that installs the native fixture, about
// half a second.
func TestAnArchiveSucceedsThoughItsDeviceGoesAwayDuringTheRelease(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithNativeFixture()).StartSetUp()
	laptop := b.Pair(master, "laptop")
	for _, id := range []string{convFirst, convSecond} {
		b.CreateConversation(master, id)
		b.SwitchTo(master, id, laptop, laptop.Home())
	}
	// The fixture service holds the conversation, and its release of it never
	// ends by itself.
	_, code, reason := b.OpenStream(master, convFirst, "stall_release").Received()
	if code != 1000 || reason != "completed" {
		t.Fatalf("the stream closes with %d %s", code, reason)
	}

	// The archive's release reaches the device, which goes away before it answers;
	// the archive succeeds all the same.
	archived := make(chan *backendtest.Answer, 1)
	failed := make(chan error, 1)
	go func() {
		answer, err := b.TryDo(backendtest.Request{
			Method: http.MethodPatch, Path: "/api/conversations/" + convFirst, Session: master,
			Body: backendtest.Map{"archived": true},
		})
		if err != nil {
			failed <- err
			return
		}
		archived <- answer
	}()
	_, code, reason = b.OpenStream(master, convSecond, "stalled").Received()
	if code != 1000 || reason != "completed" {
		t.Fatalf("the stream closes with %d %s", code, reason)
	}
	laptop.Runner.Kill()
	select {
	case answer := <-archived:
		answer.Expect(http.StatusOK)
	case err := <-failed:
		t.Fatal(err)
	}
	b.Stop()
}
