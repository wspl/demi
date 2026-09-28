package commandservice_test

import (
	"context"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"strings"
	"testing"
	"time"

	cs "github.com/wspl/demi/go/commandservice"
)

type faultHandler struct {
	completion     *cs.Completion
	releaseStarted chan struct{}
	closed         chan struct{}
}

func (h *faultHandler) Operations() []string { return []string{"panic", "cancelled", "ping"} }

func (h *faultHandler) Invoke(call *cs.Call) (cs.Completion, error) {
	switch call.Invocation.Operation {
	case "panic":
		panic("handler panic")
	case "cancelled":
		return cs.Completion{}, cs.ErrCancelled
	}
	return cs.Completion{}, nil
}

func (h *faultHandler) Conversation(call *cs.ConversationCall) (cs.Completion, error) {
	if h.completion != nil {
		return *h.completion, nil
	}
	close(h.releaseStarted)
	<-call.Context().Done()
	return cs.Completion{}, errors.New("cleanup after cancellation failed")
}

func (h *faultHandler) Close(context.Context) error {
	if h.closed != nil {
		close(h.closed)
	}
	return nil
}

// Fault tests wait on reset, handler entry, and Serve completion; budget ten seconds.
func TestHandlerFaultsPreserveConnection(t *testing.T) {
	client, ctx := clientFor(t, &faultHandler{})
	for _, operation := range []string{"panic", "cancelled"} {
		stream, err := client.Invoke(ctx, invocation(operation))
		if err != nil {
			t.Fatal(err)
		}
		if record, err := stream.Next(); err == nil {
			t.Fatalf("fault sent record: %+v", record)
		}
		run(t, ctx, client, invocation("ping"), nil)
	}
}

func TestCancelledReleaseFailureRetiresService(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	a, b := net.Pipe()
	t.Cleanup(func() {
		// Both endpoints are also owned by the SDK.
		_ = a.Close()
		_ = b.Close()
	})
	handler := &faultHandler{releaseStarted: make(chan struct{}), closed: make(chan struct{})}
	done := make(chan error, 1)
	go func() { done <- cs.Serve(ctx, a, handler) }()
	client, err := cs.Connect(ctx, b)
	if err != nil {
		t.Fatal(err)
	}
	stream, err := client.Conversation(ctx, cs.ConversationRequest{Operation: "release", Conversation: "c1"})
	if err != nil {
		t.Fatal(err)
	}
	select {
	case <-handler.releaseStarted:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	stream.Cancel()
	select {
	case err = <-done:
		if !errors.Is(err, cs.ErrConversationCleanup) || !strings.Contains(err.Error(), "cleanup after cancellation failed") {
			t.Fatal(err)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	select {
	case <-handler.closed:
	default:
		t.Fatal("Close did not run")
	}
}

func TestMalformedConnectionFailsServe(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	server, peer := net.Pipe()
	t.Cleanup(func() {
		// The malformed protocol deliberately makes both sides close independently.
		_ = server.Close()
		_ = peer.Close()
	})
	done := make(chan error, 1)
	go func() { done <- cs.Serve(ctx, server, &faultHandler{}) }()
	drained := make(chan struct{})
	go func() {
		// Read until the service closes its failed connection; the error is the test's stimulus.
		_, _ = io.Copy(io.Discard, peer)
		close(drained)
	}()
	preface := []byte("PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
	preface = append(preface, []byte{0, 0, 0, 4, 0, 0, 0, 0, 0}...)
	// DATA on stream zero is a connection-level protocol error.
	preface = append(preface, []byte{0, 0, 0, 0, 0, 0, 0, 0, 0}...)
	if _, err := peer.Write(preface); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("malformed connection returned success")
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	<-drained
}

func TestHeaderSettings(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	a, b := net.Pipe()
	t.Cleanup(func() {
		// This peer reads only the initial settings before closing the handshake.
		_ = a.Close()
		_ = b.Close()
	})
	done := make(chan error, 1)
	go func() { done <- cs.Serve(ctx, a, &faultHandler{}) }()
	wrote := make(chan error, 1)
	go func() {
		_, err := b.Write([]byte("PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"))
		wrote <- err
	}()
	var header [9]byte
	if _, err := io.ReadFull(b, header[:]); err != nil {
		t.Fatal(err)
	}
	length := int(header[0])<<16 | int(header[1])<<8 | int(header[2])
	payload := make([]byte, length)
	if _, err := io.ReadFull(b, payload); err != nil {
		t.Fatal(err)
	}
	found := false
	for offset := 0; offset+6 <= len(payload); offset += 6 {
		if binary.BigEndian.Uint16(payload[offset:]) == 6 {
			found = true
			if limit := binary.BigEndian.Uint32(payload[offset+2:]); limit != cs.HeaderListBytes {
				t.Fatalf("header setting %d", limit)
			}
		}
	}
	if !found {
		t.Fatal("missing header limit setting")
	}
	cancel()
	// Close interrupts the incomplete test handshake; cancellation owns the outcome.
	_ = b.Close()
	// This probe deliberately cancels an incomplete handshake; both errors are expected.
	<-wrote
	<-done
}

type invalidCatalog struct{}

func (invalidCatalog) Operations() []string { return nil }

func (invalidCatalog) Invoke(*cs.Call) (cs.Completion, error) { panic("invalid catalog was admitted") }

func TestInvalidCatalogClosesTransport(t *testing.T) {
	a, b := net.Pipe()
	t.Cleanup(func() {
		// Release the test transport even if catalog rejection is broken.
		_ = a.Close()
		_ = b.Close()
	})
	if err := b.SetReadDeadline(time.Now().Add(time.Second)); err != nil {
		t.Fatal(err)
	}
	if err := cs.Serve(context.Background(), a, invalidCatalog{}); err == nil {
		t.Fatal("invalid catalog accepted")
	}
	var data [1]byte
	if _, err := b.Read(data[:]); err != io.EOF {
		t.Fatalf("catalog rejection left connection open: %v", err)
	}
}

type cancellationWriter struct {
	started chan struct{}
	written chan error
}

func (h cancellationWriter) Operations() []string { return []string{"write"} }

func (h cancellationWriter) Invoke(call *cs.Call) (cs.Completion, error) {
	close(h.started)
	<-call.Context().Done()
	_, err := call.Stdout.Write([]byte("must not be accepted"))
	h.written <- err
	return cs.Completion{}, cs.ErrCancelled
}

func TestCancelledWriterDoesNotUseReadyQueue(t *testing.T) {
	handler := cancellationWriter{started: make(chan struct{}), written: make(chan error, 1)}
	client, ctx := clientFor(t, handler)
	stream, err := client.Invoke(ctx, invocation("write"))
	if err != nil {
		t.Fatal(err)
	}
	<-handler.started
	stream.Cancel()
	select {
	case err = <-handler.written:
		if !errors.Is(err, cs.ErrCancelled) {
			t.Fatal(err)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
}

func TestClientHeaderSettings(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	t.Cleanup(cancel)
	a, b := net.Pipe()
	t.Cleanup(func() {
		// This wire probe intentionally ends the handshake once it has read SETTINGS.
		_ = a.Close()
		_ = b.Close()
	})
	connected := make(chan struct{})
	go func() {
		client, err := cs.Connect(ctx, a)
		if err == nil {
			// The probe, rather than a command exchange, owns this connection's end.
			_ = client.Close()
		}
		close(connected)
	}()
	preface := make([]byte, 24)
	if _, err := io.ReadFull(b, preface); err != nil {
		t.Fatal(err)
	}
	var header [9]byte
	if _, err := io.ReadFull(b, header[:]); err != nil {
		t.Fatal(err)
	}
	length := int(header[0])<<16 | int(header[1])<<8 | int(header[2])
	payload := make([]byte, length)
	if _, err := io.ReadFull(b, payload); err != nil {
		t.Fatal(err)
	}
	found := false
	for offset := 0; offset+6 <= len(payload); offset += 6 {
		if binary.BigEndian.Uint16(payload[offset:]) == 6 {
			found = true
			if limit := binary.BigEndian.Uint32(payload[offset+2:]); limit != cs.HeaderListBytes {
				t.Fatalf("client header setting %d", limit)
			}
		}
	}
	if !found {
		t.Fatal("missing client header setting")
	}
	// Closing releases the connection's remaining initial WINDOW_UPDATE write.
	_ = b.Close()
	<-connected
}

// Budget ten seconds per case; service retirement exposes the cleanup diagnostic.
func TestReleaseFailureWords(t *testing.T) {
	for _, test := range []struct {
		name       string
		completion cs.Completion
		detail     string
	}{
		{"status", cs.Completion{ExitCode: 1}, "release exited with status 1"},
		{"command_error", cs.Completion{
			ExitCode: 1,
			Error:    &cs.CommandError{Code: "command_failed", Message: "cleanup failed"},
		}, "release exited with status 1: command_failed: cleanup failed"},
	} {
		t.Run(test.name, func(t *testing.T) {
			ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			t.Cleanup(cancel)
			server, peer := net.Pipe()
			t.Cleanup(func() {
				// Release endpoints already owned by service and client teardown.
				_ = server.Close()
				_ = peer.Close()
			})
			done := make(chan error, 1)
			go func() { done <- cs.Serve(ctx, server, &faultHandler{completion: &test.completion}) }()
			client, err := cs.Connect(ctx, peer)
			if err != nil {
				t.Fatal(err)
			}
			stream, err := client.Conversation(ctx, cs.ConversationRequest{Operation: "release", Conversation: "c1"})
			if err != nil {
				t.Fatal(err)
			}
			// Retirement may discard the completion; only a successful one is forbidden.
			record, err := stream.Next()
			if err == nil && record.Kind == cs.Completed && record.Completion.ExitCode == 0 {
				t.Fatal("successful release completion")
			}
			select {
			case err = <-done:
				if !errors.Is(err, cs.ErrConversationCleanup) || err.Error() != cs.ErrConversationCleanup.Error()+": "+test.detail {
					t.Fatal(err)
				}
			case <-ctx.Done():
				t.Fatal(ctx.Err())
			}
		})
	}
}
