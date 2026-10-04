package backendtest

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/http"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/webapiproto"
)

// SyncChannel is a page's synchronization channel. Its test owns socket cleanup.
type SyncChannel struct{ socket *websocket.Conn }

// Sync opens the session's channel from a product origin; the first message is
// the snapshot. Test cleanup closes the socket, including on assertion failure.
func (b *TestBackend) Sync(ctx context.Context, t testing.TB, session *Session) (*SyncChannel, error) {
	t.Helper()
	channel, err := b.sync(ctx, session)
	if err != nil {
		return nil, err
	}
	t.Cleanup(func() {
		if err := channel.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return channel, nil
}

// sync opens a fixture socket whose caller assumes cleanup ownership.
func (b *TestBackend) sync(ctx context.Context, session *Session) (*SyncChannel, error) {
	headers := http.Header{"Origin": []string{b.URL}, "Cookie": []string{session.Cookie}}
	socket, response, err := websocket.Dial(
		ctx,
		b.WSURL("/api/sync"),
		&websocket.DialOptions{HTTPClient: b.HTTP, HTTPHeader: headers},
	)
	if err != nil {
		if response != nil && response.Body != nil {
			err = errors.Join(err, response.Body.Close())
		}
		return nil, err
	}
	// A test sync channel accepts messages up to 64 MiB.
	socket.SetReadLimit(64 << 20)
	return &SyncChannel{socket: socket}, nil
}

// Next returns the next validated message, including heartbeats.
func (s *SyncChannel) Next(ctx context.Context) (webapiproto.SyncEvent, error) {
	for {
		kind, data, err := s.socket.Read(ctx)
		if err != nil {
			return nil, err
		}
		if kind == websocket.MessageText {
			return webapiproto.DecodeSyncEvent(data)
		}
	}
}

// Snapshot reads the channel's first message as product state.
func (s *SyncChannel) Snapshot(ctx context.Context) (webapiproto.ProductState, error) {
	event, err := s.Next(ctx)
	if err != nil {
		return webapiproto.ProductState{}, err
	}
	snapshot, ok := event.(*webapiproto.SyncEventSnapshot)
	if !ok {
		return webapiproto.ProductState{}, fmt.Errorf("the channel's first message is not the snapshot: %T", event)
	}
	return snapshot.State, nil
}

// Until reads up to and including the first message accepted by done.
func (s *SyncChannel) Until(
	ctx context.Context,
	done func(webapiproto.SyncEvent) bool,
) ([]webapiproto.SyncEvent, error) {
	var received []webapiproto.SyncEvent
	for {
		event, err := s.Next(ctx)
		if err != nil {
			return received, err
		}
		received = append(received, event)
		if done(event) {
			return received, nil
		}
	}
}

// Closed reads past pending messages to the server's close code and reason.
func (s *SyncChannel) Closed(ctx context.Context) (uint16, string, error) {
	for {
		_, _, err := s.socket.Read(ctx)
		if err == nil {
			continue
		}
		var closed websocket.CloseError
		if errors.As(err, &closed) {
			return uint16(closed.Code), closed.Reason, nil
		}
		return 0, "", err
	}
}

// SendText sends a client text message, which a page never sends on this socket.
func (s *SyncChannel) SendText(ctx context.Context, text string) error {
	return s.socket.Write(ctx, websocket.MessageText, []byte(text))
}

// Close closes the socket and releases its resources.
func (s *SyncChannel) Close(_ context.Context) error {
	err := s.socket.CloseNow()
	if errors.Is(err, net.ErrClosed) {
		return nil
	}
	return err
}
