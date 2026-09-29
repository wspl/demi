package backendtest_test

import (
	"bytes"
	"net/http"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
)

// streamConversation is a conversation the test creates under this id.
const streamConversation = "5d4c3b2a-8f3a-4c1e-9d2b-7a1c2e3f4a01"

// onCloud is a conversation that works on the Cloud.
const onCloud = "5d4c3b2a-8f3a-4c1e-9d2b-7a1c2e3f4a02"

// deviceConversation starts the harness's backend, signs the master in, pairs a
// device, and makes a conversation run in the home of that device.
func deviceConversation(t *testing.T, h *backendtest.Harness) (*backendtest.Backend, *backendtest.Session, *backendtest.Paired) {
	t.Helper()
	b, master := h.StartSetUp()
	b.Post("/api/conversations", master, backendtest.Map{"id": streamConversation}).Expect(http.StatusCreated)
	laptop := b.Pair(master, "laptop")
	target := backendtest.Map{"target": backendtest.Map{"kind": "device", "deviceId": laptop.ID(), "path": laptop.Home()}}
	b.Patch("/api/conversations/"+streamConversation, master, target).Expect(http.StatusOK)
	return b, master, laptop
}

func streamPath(conversation, name string) string {
	return "/api/conversations/" + conversation + "/streams/" + name
}

// streamRefusal is how the route answers an upgrade before upgrading, from
// origin.
func streamRefusal(b *backendtest.Backend, session *backendtest.Session, name, origin string) (int, string) {
	answer := b.Do(backendtest.Request{
		Path: streamPath(streamConversation, name), Session: session,
		Headers: map[string]string{
			"Upgrade": "websocket", "Connection": "Upgrade", "Sec-WebSocket-Version": "13",
			"Sec-WebSocket-Key": "MDEyMzQ1Njc4OWFiY2RlZg==", "Origin": origin,
		},
	})
	return answer.Refusal()
}

// Cost: one backend and a real runner, about a second: the runner installs the
// native fixture package from the backend, and three megabytes go through the
// echo.
func TestAPageOpensAUserStreamOnTheConversationsHostWithTheUsersContextAndDirectory(t *testing.T) {
	t.Parallel()
	b, master, laptop := deviceConversation(t, backendtest.New(t, backendtest.WithNativeFixture()))
	place := b.OpenStream(master, streamConversation, "where")
	data, code, reason := place.Received()
	if code != 1000 || reason != "completed" {
		t.Fatalf("the stream closes with %d %s", code, reason)
	}
	report := decodeJSON(t, data)
	if backendtest.At(report, "context.conversation") != streamConversation ||
		backendtest.At(report, "context.caller.kind") != "user" ||
		backendtest.At(report, "cwd") != laptop.Home() ||
		backendtest.At(report, "value") != nil {
		t.Fatalf("the service reports %s", data)
	}
	backendtest.AssertJSON(t, backendtest.At(report, "context.locale"), backendtest.Map{"timeZone": "UTC", "languages": []string{"en-US"}})
	// Once the user's browser reports a locale, the context carries it.
	locale := backendtest.Map{"timeZone": "Asia/Shanghai", "languages": []string{"zh-CN", "en"}}
	b.Patch("/api/settings/preferences", master, backendtest.Map{"locale": locale}).Expect(http.StatusOK)
	data, _, _ = b.OpenStream(master, streamConversation, "where").Received()
	backendtest.AssertJSON(t, backendtest.At(decodeJSON(t, data), "context.locale"), locale)

	// Bytes go both ways as they are, in order, whatever the message
	// boundaries; the page closing its socket ends the stream.
	echo := b.OpenStream(master, streamConversation, "echo")
	payload := backendtest.Pattern(3*1024*1024, 0)
	var sending sync.WaitGroup
	sending.Add(1)
	go func() {
		defer sending.Done()
		for start := 0; start < len(payload); start += 100_000 {
			if err := echo.TrySendBinary(payload[start:min(start+100_000, len(payload))]); err != nil {
				t.Error(err)
				return
			}
		}
	}()
	echoed := echo.ReadBytes(len(payload))
	sending.Wait()
	if !bytes.Equal(echoed, payload) {
		t.Fatal("the echo differs from what was sent")
	}
	echo.Close()
	b.Stop()
}

// Cost: one backend and a real runner, about half a second; the runner is killed and
// started again.
func TestTheRouteRefusesAForeignOriginAnUnknownStreamAnArchivedConversationAndAnOfflineDevice(t *testing.T) {
	t.Parallel()
	b, master, laptop := deviceConversation(t, backendtest.New(t, backendtest.WithNativeFixture()))
	product := b.URL
	for _, refusal := range []struct {
		name, origin string
		status       int
		code         string
	}{
		{"echo", "https://elsewhere.example", http.StatusForbidden, "forbidden_origin"},
		{"browser", product, http.StatusNotFound, "unknown_stream"},
	} {
		if status, code := streamRefusal(b, master, refusal.name, refusal.origin); status != refusal.status || code != refusal.code {
			t.Fatalf("%s from %s is %d %s", refusal.name, refusal.origin, status, code)
		}
	}
	wantRefusal(t, b.Get(streamPath(streamConversation, "echo"), master), http.StatusUpgradeRequired, "upgrade_required", "a plain request")

	laptop.Runner.Kill()
	b.UntilOnline(master, laptop.ID(), false)
	if status, code := streamRefusal(b, master, "echo", product); status != http.StatusConflict || code != "device_offline" {
		t.Fatalf("a stream to an offline device is %d %s", status, code)
	}
	if err := laptop.Runner.StartAgain(); err != nil {
		t.Fatal(err)
	}
	b.UntilOnline(master, laptop.ID(), true)

	b.Patch("/api/conversations/"+streamConversation, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	if status, code := streamRefusal(b, master, "echo", product); status != http.StatusConflict || code != "conversation_archived" {
		t.Fatalf("a stream of an archived conversation is %d %s", status, code)
	}
	b.Stop()
}

// Cost: one backend and a real runner, about half a second.
func TestAnArchiveEndsTheConversationsOpenStreams(t *testing.T) {
	t.Parallel()
	b, master, _ := deviceConversation(t, backendtest.New(t, backendtest.WithNativeFixture()))
	echo := b.OpenStream(master, streamConversation, "echo")
	echo.Answered()
	b.Patch("/api/conversations/"+streamConversation, master, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	_, code, reason := echo.Received()
	if code != 4000 || reason != "conversation_changed" {
		t.Fatalf("the stream closes with %d %s", code, reason)
	}
	b.Stop()
}

// The Cloud boots, and three idle windows pass in real time: the stream stays
// open past the window a file read started, so that a stream that did not count
// would let the Cloud stop while it is open, and the Cloud idles for one after
// the close.
//
// Cost: one backend, a scripted manager and a real runner, about two seconds.
func TestAnOpenStreamKeepsTheCloudItWatchesAwakeUntilItCloses(t *testing.T) {
	t.Parallel()
	const window = 400 * time.Millisecond
	h := backendtest.New(t, backendtest.WithNativeFixture())
	idleAfter(h, window)
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	b, master := h.StartSetUp()
	b.Post("/api/conversations", master, backendtest.Map{"id": onCloud}).Expect(http.StatusCreated)
	// Reading the conversation's files wakes the Cloud it works on.
	b.Get("/api/conversations/"+onCloud+"/fs", master).Expect(http.StatusOK)
	device := theCloud(t, h)
	echo := b.OpenStream(master, onCloud, "echo")
	echo.Answered()

	// Watched for two idle windows, the Cloud stays up: it stops a full window
	// after the page closes the stream, and not before.
	time.Sleep(window * 2)
	closed := time.Now()
	echo.Close()
	stopped := h.Manager.Arrival("hibernate:"+device, backendtest.Patience)
	if stopped.Before(closed.Add(window)) {
		t.Fatalf("the Cloud stopped %s after the stream closed", stopped.Sub(closed))
	}
	b.Stop()
}

// idleAfter sets short idle windows, read often, and no retention pass by
// itself.
func idleAfter(h *backendtest.Harness, window time.Duration) {
	lifecycle := h.Lifecycle()
	lifecycle.IdleWindowMs = backendtest.Ptr(uint64(window.Milliseconds()))
	lifecycle.IdlePollMs = backendtest.Ptr(uint64(50))
	lifecycle.RetentionIntervalMs = backendtest.Ptr(uint64(0))
}

// theCloud is the one device the manager made, the user's Cloud.
func theCloud(t *testing.T, h *backendtest.Harness) string {
	t.Helper()
	devices := h.Manager.Devices()
	if len(devices) != 1 {
		t.Fatalf("the manager made %v", devices)
	}
	return devices[0]
}

// decodeJSON decodes a document, or fails the test.
func decodeJSON(t *testing.T, data []byte) any {
	t.Helper()
	return backendtest.Decode(t, data)
}
