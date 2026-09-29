package backendtest

import (
	"context"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
)

// A Frame is a message of a socket, as loose JSON: its type is the "type"
// member.
type Frame = map[string]any

// maxMessageBytes is the largest message a scenario's socket reads.
const maxMessageBytes = 64 << 20

// dial opens a WebSocket to path from a page at origin with the session's
// cookie, and answers the connection, or the status and body of the answer that
// refused the upgrade.
func (b *Backend) dial(session *Session, path, origin string) (*websocket.Conn, int, []byte, error) {
	header := http.Header{}
	if session != nil {
		header.Set("Cookie", session.Cookie)
	}
	if origin != "" {
		header.Set("Origin", origin)
	}
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	conn, response, err := websocket.Dial(ctx, "ws://"+strings.TrimPrefix(b.URL, "http://")+path, &websocket.DialOptions{
		HTTPHeader: header,
	})
	if err != nil {
		if response != nil {
			defer response.Body.Close()
			body, _ := io.ReadAll(response.Body)
			return nil, response.StatusCode, body, err
		}
		return nil, 0, nil, err
	}
	conn.SetReadLimit(maxMessageBytes)
	return conn, http.StatusSwitchingProtocols, nil, nil
}

// A Socket is a page's conversation socket (runtime.md § Frame protocol).
type Socket struct {
	t    testing.TB
	conn *websocket.Conn
	// Patience is how long it waits for the next message before the scenario
	// fails as hung: ten seconds, longer where a real Cloud boots within a turn.
	Patience time.Duration
}

func (b *Backend) newSocket(conn *websocket.Conn) *Socket {
	s := &Socket{t: b.t, conn: conn, Patience: 10 * time.Second}
	b.t.Cleanup(func() {
		// A socket the scenario left open ends with the test.
		_ = conn.CloseNow()
	})
	return s
}

// Connect opens the conversation's socket from a page of the product, or fails
// the test when the route refuses the upgrade.
func (b *Backend) Connect(session *Session, conversation string) *Socket {
	b.t.Helper()
	socket, status, code := b.TryConnectFrom(session, conversation, b.URL)
	if socket == nil {
		b.t.Fatalf("the stream refused the upgrade with %d %s", status, code)
	}
	return socket
}

// TryConnect opens the conversation's socket from a page of the product, or
// answers the status and error code the route gave instead of upgrading.
func (b *Backend) TryConnect(session *Session, conversation string) (*Socket, int, string) {
	b.t.Helper()
	return b.TryConnectFrom(session, conversation, b.URL)
}

// TryConnectFrom opens the conversation's socket from a page at origin, or
// answers the status and error code the route gave instead of upgrading.
func (b *Backend) TryConnectFrom(session *Session, conversation, origin string) (*Socket, int, string) {
	b.t.Helper()
	conn, status, body, err := b.dial(session, "/api/conversations/"+conversation+"/stream", origin)
	if conn == nil {
		if status == 0 {
			b.t.Fatalf("the socket did not connect: %v", err)
		}
		return nil, status, (&Answer{Body: body}).ErrorCode()
	}
	return b.newSocket(conn), status, ""
}

// Send sends a frame.
func (s *Socket) Send(frame Frame) {
	s.t.Helper()
	s.SendText(string(Marshal(frame)))
}

// SendText sends a text message, which a page never does with anything but a
// frame.
func (s *Socket) SendText(text string) {
	s.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	if err := s.conn.Write(ctx, websocket.MessageText, []byte(text)); err != nil {
		s.t.Fatalf("the socket does not take a message: %v", err)
	}
}

// receive reads the next message: a frame, or the close code when the socket
// closed (-1 when it ended without one).
func (s *Socket) receive() (frame Frame, closeCode int, closed bool) {
	s.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), s.Patience)
	defer cancel()
	for {
		kind, data, err := s.conn.Read(ctx)
		if err != nil {
			if ctx.Err() != nil {
				s.t.Fatalf("the socket delivers within %s", s.Patience)
			}
			if code := websocket.CloseStatus(err); code != -1 {
				return nil, int(code), true
			}
			return nil, -1, true
		}
		if kind != websocket.MessageText {
			continue
		}
		if err := json.Unmarshal(data, &frame); err != nil {
			s.t.Fatalf("the frame is not JSON: %v: %s", err, data)
		}
		return frame, 0, false
	}
}

// Frame reads the next frame, and fails the test when the socket closed.
func (s *Socket) Frame() Frame {
	s.t.Helper()
	frame, code, closed := s.receive()
	if closed {
		s.t.Fatalf("the socket closed with %d", code)
	}
	return frame
}

// Until reads the frames up to and including the first that done accepts.
func (s *Socket) Until(done func(Frame) bool) []Frame {
	s.t.Helper()
	var frames []Frame
	for {
		frame := s.Frame()
		frames = append(frames, frame)
		if done(frame) {
			return frames
		}
	}
}

// UntilType reads the frames up to and including the first of the given type.
func (s *Socket) UntilType(kind string) []Frame {
	s.t.Helper()
	return s.Until(func(frame Frame) bool { return frame["type"] == kind })
}

// Open opens the conversation, with the model its record holds, and reads the
// handshake.
func (s *Socket) Open() []Frame {
	s.t.Helper()
	s.Send(Frame{"type": "open"})
	handshake := s.UntilType("pending_steers")
	if handshake[0]["type"] != "opened" {
		s.t.Fatalf("the handshake opens with %v", handshake[0])
	}
	return handshake
}

// SendMessage is the frame that sends a message of one text part.
func SendMessage(id, text string) Frame {
	return Frame{"type": "send", "messageId": id, "content": []any{Frame{"type": "text", "text": text}}}
}

// Chat sends a message and reads the frames of its turn, to the phase that says
// it ended.
func (s *Socket) Chat(id, text string) []Frame {
	s.t.Helper()
	s.Send(SendMessage(id, text))
	return s.UntilIdle()
}

// IsPhase reports whether the frame is the phase frame of phase.
func IsPhase(frame Frame, phase string) bool {
	return frame["type"] == "phase" && frame["phase"] == phase
}

// UntilIdle reads the frames up to the idle phase that follows a running one.
func (s *Socket) UntilIdle() []Frame {
	s.t.Helper()
	ran := false
	return s.Until(func(frame Frame) bool {
		if IsPhase(frame, "running") {
			ran = true
			return false
		}
		return ran && IsPhase(frame, "idle")
	})
}

// Stop stops the running turn, and reads its frames to both the answer of the
// stop and the idle phase, which arrive in either order.
func (s *Socket) Stop() {
	s.t.Helper()
	s.Send(Frame{"type": "abort"})
	answered, idle := false, false
	for !(answered && idle) {
		frame := s.Frame()
		switch {
		case frame["type"] == "abort_result":
			answered = true
		case IsPhase(frame, "idle"):
			idle = true
		}
	}
}

// Live returns the live transcript's blocks, as a fresh reset sends them.
func (s *Socket) Live() []any {
	s.t.Helper()
	s.Send(Frame{"type": "sync_transcript"})
	frames := s.UntilType("transcript_reset")
	blocks, _ := frames[len(frames)-1]["blocks"].([]any)
	return blocks
}

// Closed reads to the close code, skipping the frames before it; -1 when the
// socket ended without one.
func (s *Socket) Closed() int {
	s.t.Helper()
	for {
		frame, code, closed := s.receive()
		_ = frame
		if closed {
			return code
		}
	}
}

// Close closes the socket from the page's side.
func (s *Socket) Close() {
	// A socket that already ended has nothing to close.
	_ = s.conn.Close(websocket.StatusNormalClosure, "")
}

// A SyncChannel is a page's synchronization channel (web-api.md § Page
// synchronization).
type SyncChannel struct {
	t    testing.TB
	conn *websocket.Conn
}

// Sync opens the session's synchronization channel from a page of the product;
// its first message is the snapshot.
func (b *Backend) Sync(session *Session) *SyncChannel {
	b.t.Helper()
	conn, status, body, err := b.dial(session, "/api/sync", b.URL)
	if conn == nil {
		b.t.Fatalf("the channel refused the upgrade with %d: %v %s", status, err, body)
	}
	b.t.Cleanup(func() {
		// A channel the scenario left open ends with the test.
		_ = conn.CloseNow()
	})
	return &SyncChannel{t: b.t, conn: conn}
}

// TrySync opens the channel from a page at origin, or answers the status and
// error code the route gave instead of upgrading.
func (b *Backend) TrySync(session *Session, origin string) (*SyncChannel, int, string) {
	b.t.Helper()
	conn, status, body, err := b.dial(session, "/api/sync", origin)
	if conn == nil {
		if status == 0 {
			b.t.Fatalf("the channel did not connect: %v", err)
		}
		return nil, status, (&Answer{Body: body}).ErrorCode()
	}
	b.t.Cleanup(func() {
		_ = conn.CloseNow()
	})
	return &SyncChannel{t: b.t, conn: conn}, status, ""
}

// Next reads the next message, a heartbeat included.
func (c *SyncChannel) Next() Frame {
	c.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	for {
		kind, data, err := c.conn.Read(ctx)
		if err != nil {
			c.t.Fatalf("the channel ended: %v", err)
		}
		if kind != websocket.MessageText {
			continue
		}
		var event Frame
		if err := json.Unmarshal(data, &event); err != nil {
			c.t.Fatalf("the event is not JSON: %v: %s", err, data)
		}
		return event
	}
}

// Snapshot reads the product state, the channel's first message.
func (c *SyncChannel) Snapshot() Frame {
	c.t.Helper()
	event := c.Next()
	if event["type"] != "snapshot" {
		c.t.Fatalf("the channel's first message is not the snapshot: %v", event)
	}
	state, _ := event["state"].(map[string]any)
	return state
}

// Until reads the messages up to and including the first that done accepts.
func (c *SyncChannel) Until(done func(Frame) bool) []Frame {
	c.t.Helper()
	var received []Frame
	for {
		event := c.Next()
		received = append(received, event)
		if done(event) {
			return received
		}
	}
}

// Closed reads to the code and reason the backend closed the channel with,
// after the messages still on their way.
func (c *SyncChannel) Closed() (int, string) {
	c.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	for {
		_, _, err := c.conn.Read(ctx)
		if err == nil {
			continue
		}
		var closed websocket.CloseError
		if errors.As(err, &closed) {
			return int(closed.Code), closed.Reason
		}
		c.t.Fatalf("the channel ended without a close: %v", err)
	}
}

// Close ends the channel from the page's side.
func (c *SyncChannel) Close() {
	// A channel that ended already has nothing to close.
	_ = c.conn.Close(websocket.StatusNormalClosure, "")
}

// SendText sends the backend a text message, which a page never does.
func (c *SyncChannel) SendText(text string) {
	c.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	if err := c.conn.Write(ctx, websocket.MessageText, []byte(text)); err != nil {
		c.t.Fatalf("the channel does not take a message: %v", err)
	}
}

// A Stream is a page's socket on a user stream of a conversation's Host
// (web-api.md § User streams): binary messages both ways.
type Stream struct {
	t    testing.TB
	conn *websocket.Conn
}

// OpenStream opens the user stream name of the conversation from a page of the
// product.
func (b *Backend) OpenStream(session *Session, conversation, name string) *Stream {
	b.t.Helper()
	conn, status, body, err := b.dial(session, "/api/conversations/"+conversation+"/streams/"+name, b.URL)
	if conn == nil {
		b.t.Fatalf("the stream %s refused the upgrade with %d: %v %s", name, status, err, body)
	}
	b.t.Cleanup(func() {
		_ = conn.CloseNow()
	})
	return &Stream{t: b.t, conn: conn}
}

// SendBinary sends bytes as one message.
func (s *Stream) SendBinary(data []byte) {
	s.t.Helper()
	if err := s.TrySendBinary(data); err != nil {
		s.t.Fatal(err)
	}
}

// TrySendBinary sends bytes as SendBinary does and answers an error instead of
// failing the test, for a goroutine of the scenario.
func (s *Stream) TrySendBinary(data []byte) error {
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	if err := s.conn.Write(ctx, websocket.MessageBinary, data); err != nil {
		return fmt.Errorf("the stream does not take a message: %w", err)
	}
	return nil
}

// Received reads what the page receives until the backend closes the socket,
// and answers the bytes and the close's code and reason.
func (s *Stream) Received() ([]byte, int, string) {
	s.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	var data []byte
	for {
		kind, chunk, err := s.conn.Read(ctx)
		if err != nil {
			var closed websocket.CloseError
			if errors.As(err, &closed) {
				return data, int(closed.Code), closed.Reason
			}
			s.t.Fatalf("the stream ended without a close: %v", err)
		}
		if kind == websocket.MessageBinary {
			data = append(data, chunk...)
		}
	}
}

// ReadBytes reads until count bytes arrived, and answers them.
func (s *Stream) ReadBytes(count int) []byte {
	s.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	var data []byte
	for len(data) < count {
		kind, chunk, err := s.conn.Read(ctx)
		if err != nil {
			s.t.Fatalf("the stream ended after %d of %d bytes: %v", len(data), count, err)
		}
		if kind == websocket.MessageBinary {
			data = append(data, chunk...)
		}
	}
	return data
}

// Answered waits until the echo service answers, which tells that the stream is
// open.
func (s *Stream) Answered() {
	s.t.Helper()
	s.SendBinary([]byte("ping"))
	if got := s.ReadBytes(4); string(got) != "ping" {
		s.t.Fatalf("the echo answers %q", got)
	}
}

// Close closes the stream from the page's side.
func (s *Stream) Close() {
	// A stream that already ended has nothing to close.
	_ = s.conn.Close(websocket.StatusNormalClosure, "")
}

// Pattern returns length bytes that seed varies, which repeat only every 64,256
// bytes, so that a read from a shifted offset shows.
func Pattern(length int, seed byte) []byte {
	const period = 251 * 256
	unit := make([]byte, period)
	for index := range unit {
		unit[index] = byte((index*31+(index>>8))%251) ^ seed
	}
	bytes := make([]byte, 0, length)
	for len(bytes) < length {
		bytes = append(bytes, unit[:min(length-len(bytes), period)]...)
	}
	return bytes
}
