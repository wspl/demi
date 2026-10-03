package backendtest

//revive:disable:unused-parameter

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/webapi"
)

// SyncChannel is a page's synchronization channel. Its test owns socket cleanup.
type SyncChannel struct{}

// Sync opens the session's channel from a product origin; the first message is
// the snapshot. Test cleanup closes the socket, including on assertion failure.
func (b *TestBackend) Sync(ctx context.Context, t testing.TB, session *Session) (*SyncChannel, error) {
	panic("not written: b-backend")
}

// Next returns the next validated message, including heartbeats.
func (s *SyncChannel) Next(ctx context.Context) (webapi.SyncEvent, error) {
	panic("not written: b-backend")
}

// Snapshot reads the channel's first message as product state.
func (s *SyncChannel) Snapshot(ctx context.Context) (webapi.ProductState, error) {
	panic("not written: b-backend")
}

// Until reads up to and including the first message accepted by done.
func (s *SyncChannel) Until(ctx context.Context, done func(webapi.SyncEvent) bool) ([]webapi.SyncEvent, error) {
	panic("not written: b-backend")
}

// Closed reads past pending messages to the server's close code and reason.
func (s *SyncChannel) Closed(ctx context.Context) (uint16, string, error) {
	panic("not written: b-backend")
}

// SendText sends a client text message, which a page never sends on this socket.
func (s *SyncChannel) SendText(ctx context.Context, text string) error {
	panic("not written: b-backend")
}

// Close closes the socket and releases its resources.
func (s *SyncChannel) Close(ctx context.Context) error { panic("not written: b-backend") }
