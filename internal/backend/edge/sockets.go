package edge

import (
	"context"
	"encoding/base64"
	"errors"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/usershard"
)

// pageHandshake refuses an invalid page upgrade before admission can wake a
// Host. coder/websocket lacks a validate-only entry point, so this mirrors
// Accept's server request checks (verifyClientRequest in v1.8.15). Origin is
// already checked by the session gate; Accept uses InsecureSkipVerify.
func pageHandshake(r *http.Request, description string) error {
	if !r.ProtoAtLeast(1, 1) || r.Method != http.MethodGet ||
		!websocketToken(r.Header, "Connection", "upgrade") ||
		!websocketToken(r.Header, "Upgrade", "websocket") ||
		r.Header.Get("Sec-WebSocket-Version") != "13" {
		return apiFailure(426, "upgrade_required", description)
	}
	keys := r.Header.Values("Sec-WebSocket-Key")
	if len(keys) != 1 {
		return apiFailure(426, "upgrade_required", description)
	}
	key, err := base64.StdEncoding.DecodeString(strings.TrimSpace(keys[0]))
	if err != nil || len(key) != 16 {
		return apiFailure(426, "upgrade_required", description)
	}
	return nil
}

// websocketToken recognizes a handshake token across comma lists and repeated
// header lines, with the same case and whitespace rules as coder/websocket.
func websocketToken(header http.Header, name, token string) bool {
	for _, value := range header.Values(name) {
		for part := range strings.SplitSeq(value, ",") {
			if strings.EqualFold(strings.TrimSpace(part), token) {
				return true
			}
		}
	}
	return false
}

func pageUpgrade(w http.ResponseWriter, r *http.Request, description string) (*websocket.Conn, error) {
	if err := pageHandshake(r, description); err != nil {
		return nil, err
	}
	socket, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
	if err != nil {
		return nil, err
	}
	return socket, nil
}

func (e *Edge) syncChannel(w http.ResponseWriter, r *http.Request) error {
	cookie, err := r.Cookie("demi_session")
	if err != nil {
		return unauthenticated()
	}
	token := database.HashToken(cookie.Value)
	session, found, err := e.state.Services.Sessions.Check(r.Context(), token)
	if err != nil {
		return err
	}
	if !found {
		return unauthenticated()
	}
	socket, err := pageUpgrade(w, r, "The synchronization channel is a WebSocket")
	if err != nil {
		return err
	}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = socket.CloseNow() }()
	shard, err := e.state.Shards.Of(r.Context(), session.User.ID)
	if err != nil {
		return nil
	}
	return shard.ServeSyncChannel(
		r.Context(),
		socket,
		usershard.ChannelSession{Token: token, User: session.User, ExpiresAt: session.ExpiresAt},
	)
}

func (e *Edge) conversationSocket(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	if record.Archived {
		return apiFailure(409, "conversation_archived", "Use the transcript for an archived conversation's history")
	}
	socket, err := pageUpgrade(w, r, "The conversation stream is a WebSocket")
	if err != nil {
		return err
	}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = socket.CloseNow() }()
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return nil
	}
	return shard.ServeConversationSocket(r.Context(), *record, socket)
}

func (e *Edge) userStream(w http.ResponseWriter, r *http.Request) error {
	name := r.PathValue("name")
	unknown := apiFailure(404, "unknown_stream", "No stream has that name")
	binding, ok := e.state.Services.UserStreams.Lookup(name)
	if !ok {
		return unknown
	}
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	if err := pageHandshake(r, "A user stream is a WebSocket"); err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	pluginOff, err := shard.Plugins().StreamEnd(r.Context(), name)
	if err != nil {
		return err
	}
	if pluginOff == nil {
		return unknown
	}
	stream, err := hostaccess.OpenUserStream(r.Context(), shard.HostShard(), record.ID, binding)
	if err != nil {
		return err
	}
	defer stream.Lease.Release()
	defer stream.ToHost.Fail("user stream ended")
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = stream.FromHost.Close(context.WithoutCancel(r.Context())) }()
	socket, err := pageUpgrade(w, r, "A user stream is a WebSocket")
	if err != nil {
		return err
	}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = socket.CloseNow() }()
	relayUserStream(r.Context(), socket, stream, pluginOff, e.state.Services.Pages.CloseWait)
	return nil
}

type streamEnd struct {
	code   websocket.StatusCode
	reason string
}

func relayUserStream(
	ctx context.Context,
	socket *websocket.Conn,
	stream *hostaccess.UserStream,
	pluginOff <-chan struct{},
	closeWait time.Duration,
) {
	copyCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	outcomes := make(chan streamEnd, 2)
	joined := make(chan struct{})
	go func() {
		defer close(joined)
		copyStreamFromHost(copyCtx, socket, stream, outcomes)
	}()
	forwarded := make(chan struct{})
	go func() {
		defer close(forwarded)
		copyStreamToHost(copyCtx, socket, stream, outcomes)
	}()
	var end streamEnd
	select {
	case <-stream.Lease.Context().Done():
		end = streamEnd{4000, "conversation_changed"}
	case <-pluginOff:
		end = streamEnd{4001, "plugin_disabled"}
	case end = <-outcomes:
	case <-ctx.Done():
	}
	// Revocation wins a pipe failure caused by that same revocation.
	if stream.Lease.Context().Err() != nil {
		end = streamEnd{4000, "conversation_changed"}
	}
	if end.code != 0 {
		closed := make(chan struct{})
		timer := time.AfterFunc(closeWait, func() {
			defer close(closed)
			_ = socket.CloseNow()
		})
		_ = socket.Close(end.code, end.reason)
		if !timer.Stop() {
			<-closed
		}
	}
	cancel()
	_ = socket.CloseNow()
	<-joined
	<-forwarded
}

func copyStreamFromHost(
	ctx context.Context,
	socket *websocket.Conn,
	stream *hostaccess.UserStream,
	outcomes chan<- streamEnd,
) {
	for {
		bytes, err := stream.FromHost.Next(ctx)
		if errors.Is(err, io.EOF) {
			outcomes <- streamEnd{1000, "completed"}
			return
		}
		if err != nil {
			outcomes <- streamEnd{1011, "host_unreachable"}
			return
		}
		if err := socket.Write(ctx, websocket.MessageBinary, bytes); err != nil {
			outcomes <- streamEnd{}
			return
		}
	}
}

func copyStreamToHost(
	ctx context.Context,
	socket *websocket.Conn,
	stream *hostaccess.UserStream,
	outcomes chan<- streamEnd,
) {
	for {
		kind, bytes, err := usershard.ReadPageMessage(ctx, socket)
		if err != nil {
			outcomes <- streamEnd{}
			return
		}
		if kind != websocket.MessageBinary {
			outcomes <- streamEnd{1003, "binary_only"}
			return
		}
		if err := stream.ToHost.Write(ctx, bytes); err != nil {
			outcomes <- streamEnd{1011, "host_unreachable"}
			return
		}
	}
}
