package backendtest

import (
	"bufio"
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"testing"

	"github.com/coder/websocket"
	"github.com/fsnotify/fsnotify"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// ConversationSocket is the page's typed conversation connection.
// The test owns it and closes it before the backend shuts down.
type ConversationSocket struct{ socket *websocket.Conn }

// Conversation opens a conversation stream from a product origin.
func (b *TestBackend) Conversation(ctx context.Context, t testing.TB, s *Session, id string) (*ConversationSocket, error) {
	t.Helper()
	socket, _, err := b.ConversationFrom(ctx, t, s, id, b.URL)
	return socket, err
}

// ConversationFrom opens a stream with an explicit origin, or returns its refusal.
func (b *TestBackend) ConversationFrom(ctx context.Context, t testing.TB, s *Session, id, origin string) (*ConversationSocket, Answer, error) {
	t.Helper()
	headers := http.Header{"Origin": {origin}, "Cookie": {s.Cookie}}
	socket, response, err := websocket.Dial(ctx, b.WSURL("/api/conversations/"+id+"/stream"), &websocket.DialOptions{HTTPClient: b.HTTP, HTTPHeader: headers})
	if err != nil {
		if response != nil && response.Body != nil {
			answer, readErr := ReadAnswer(ctx, response)
			return nil, answer, errors.Join(err, readErr)
		}
		return nil, Answer{}, err
	}
	socket.SetReadLimit(64 << 20)
	c := &ConversationSocket{socket: socket}
	t.Cleanup(func() {
		if err := c.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return c, Answer{}, nil
}

// Send writes a typed frame using the shared wire encoder.
func (s *ConversationSocket) Send(ctx context.Context, frame framewire.ClientFrame) error {
	data, err := contract.EncodeJSON(frame)
	if err != nil {
		return err
	}
	return s.Text(ctx, string(data))
}

// Text sends raw text, including deliberately invalid frame input.
func (s *ConversationSocket) Text(ctx context.Context, text string) error {
	return s.socket.Write(ctx, websocket.MessageText, []byte(text))
}

// Next reads and validates the next text frame.
func (s *ConversationSocket) Next(ctx context.Context) (framewire.ServerFrame, error) {
	for {
		kind, data, err := s.socket.Read(ctx)
		if err != nil {
			return nil, err
		}
		if kind == websocket.MessageText {
			return framewire.DecodeServerFrame(data)
		}
	}
}

// Until reads through the frame accepted by done.
func (s *ConversationSocket) Until(ctx context.Context, done func(framewire.ServerFrame) bool) ([]framewire.ServerFrame, error) {
	var frames []framewire.ServerFrame
	for {
		frame, err := s.Next(ctx)
		if err != nil {
			return frames, err
		}
		frames = append(frames, frame)
		if done(frame) {
			return frames, nil
		}
	}
}

// Open attaches to the persisted model and reads the complete handshake.
func (s *ConversationSocket) Open(ctx context.Context) ([]framewire.ServerFrame, error) {
	if err := s.Send(ctx, &framewire.OpenFrame{}); err != nil {
		return nil, err
	}
	frames, err := s.Until(ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.PendingSteersFrame)
		return ok
	})
	if err != nil {
		return frames, err
	}
	if _, ok := frames[0].(*framewire.OpenedFrame); !ok {
		return frames, fmt.Errorf("handshake starts with %T", frames[0])
	}
	return frames, nil
}

// UntilIdle reads a running phase followed by an idle phase.
func (s *ConversationSocket) UntilIdle(ctx context.Context) ([]framewire.ServerFrame, error) {
	ran := false
	return s.Until(ctx, func(f framewire.ServerFrame) bool {
		phase, ok := f.(*framewire.PhaseFrame)
		if !ok {
			return false
		}
		if phase.Phase == core.SessionPhaseRunning {
			ran = true
		}
		return ran && phase.Phase == core.SessionPhaseIdle
	})
}

// Chat sends a text message and follows its turn to idle.
func (s *ConversationSocket) Chat(ctx context.Context, id, text string) ([]framewire.ServerFrame, error) {
	if err := s.Send(ctx, ConversationText(id, text)); err != nil {
		return nil, err
	}
	return s.UntilIdle(ctx)
}

// Live requests the current transcript without implementing a patch applier.
func (s *ConversationSocket) Live(ctx context.Context) ([]core.Block, error) {
	if err := s.Send(ctx, &framewire.SyncTranscriptFrame{}); err != nil {
		return nil, err
	}
	frames, err := s.Until(ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.TranscriptResetFrame)
		return ok
	})
	if err != nil {
		return nil, err
	}
	return frames[len(frames)-1].(*framewire.TranscriptResetFrame).Blocks, nil
}

// Stop waits for both the abort answer and idle, in either order.
func (s *ConversationSocket) Stop(ctx context.Context) error {
	if err := s.Send(ctx, &framewire.AbortFrame{}); err != nil {
		return err
	}
	answered, idle := false, false
	_, err := s.Until(ctx, func(f framewire.ServerFrame) bool {
		if _, ok := f.(*framewire.AbortResultFrame); ok {
			answered = true
		}
		if p, ok := f.(*framewire.PhaseFrame); ok && p.Phase == core.SessionPhaseIdle {
			idle = true
		}
		return answered && idle
	})
	return err
}

// Closed reads past buffered frames to the server's close code and reason.
func (s *ConversationSocket) Closed(ctx context.Context) (websocket.StatusCode, string, error) {
	for {
		_, err := s.Next(ctx)
		if err == nil {
			continue
		}
		var closed websocket.CloseError
		if errors.As(err, &closed) {
			return closed.Code, closed.Reason, nil
		}
		return 0, "", err
	}
}

// Close releases the socket immediately, including during failed assertions.
func (s *ConversationSocket) Close(_ context.Context) error {
	err := s.socket.CloseNow()
	if errors.Is(err, net.ErrClosed) {
		return nil
	}
	return err
}

// WaitRunnerJobsRemoved waits for the runner to release all ended job directories.
func WaitRunnerJobsRemoved(ctx context.Context, stateDir string) (err error) {
	jobs := filepath.Join(stateDir, "jobs")
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, watcher.Close()) }()
	if err := watcher.Add(jobs); err != nil {
		return err
	}
	for {
		entries, err := os.ReadDir(jobs)
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
		held := false
		for _, entry := range entries {
			held = held || entry.IsDir()
		}
		if !held {
			return nil
		}
		select {
		case <-watcher.Events:
		case err := <-watcher.Errors:
			return err
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

// StallConversationReset opens a page with a tiny receive buffer and stops at
// the reset's frame header. Reading below the WebSocket API is necessary here:
// Read would consume the entire reset.
func (b *TestBackend) StallConversationReset(ctx context.Context, t testing.TB, s *Session, id string) (*StalledConversation, error) {
	t.Helper()
	conn, err := (&net.Dialer{}).DialContext(ctx, "tcp", b.Address().String())
	if err != nil {
		return nil, err
	}
	stopped := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		// Cancellation breaks pending IO; cleanup below releases the connection.
		_ = conn.Close()
		close(stopped)
	})
	t.Cleanup(func() {
		if !stop() {
			<-stopped
		}
		if err := conn.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
			t.Error(err)
		}
	})
	tcp, ok := conn.(*net.TCPConn)
	if !ok {
		return nil, errors.New("conversation transport is not TCP")
	}
	if err := tcp.SetReadBuffer(4096); err != nil {
		return nil, err
	}
	if deadline, ok := ctx.Deadline(); ok {
		if err := tcp.SetDeadline(deadline); err != nil {
			return nil, err
		}
	}
	path := "/api/conversations/" + id + "/stream"
	request, err := http.NewRequestWithContext(ctx, "GET", b.URL+path, nil)
	if err != nil {
		return nil, err
	}
	request.Header = http.Header{"Cookie": {s.Cookie}, "Origin": {b.URL}, "Connection": {"Upgrade"}, "Upgrade": {"websocket"}, "Sec-Websocket-Version": {"13"}, "Sec-Websocket-Key": {"MDEyMzQ1Njc4OWFiY2RlZg=="}}
	if err := request.Write(conn); err != nil {
		return nil, err
	}
	reader := bufio.NewReader(conn)
	response, err := http.ReadResponse(reader, request)
	if err != nil {
		return nil, err
	}
	if response.StatusCode != 101 {
		answer, readErr := ReadAnswer(ctx, response)
		return nil, errors.Join(fmt.Errorf("upgrade: %d: %s", answer.Status, answer.Body), readErr)
	}
	// The upgraded response body is the connection, which test cleanup owns.
	payload, err := contract.EncodeJSON(&framewire.OpenFrame{})
	if err != nil {
		return nil, err
	}
	frame := append([]byte{0x81, 0x80 | byte(len(payload)), 0, 0, 0, 0}, payload...)
	if _, err := conn.Write(frame); err != nil {
		return nil, err
	}
	header := make([]byte, 2)
	if _, err := io.ReadFull(reader, header); err != nil {
		return nil, err
	}
	if header[0] != 0x81 || header[1] != 17 {
		return nil, fmt.Errorf("opened frame header: %x", header)
	}
	opened := make([]byte, 17)
	if _, err := io.ReadFull(reader, opened); err != nil {
		return nil, err
	}
	if string(opened) != `{"type":"opened"}` {
		return nil, fmt.Errorf("opened: %s", opened)
	}
	if _, err := io.ReadFull(reader, header); err != nil {
		return nil, err
	}
	if header[0] != 0x81 || header[1] != 127 {
		return nil, fmt.Errorf("reset frame header: %x", header)
	}
	length := make([]byte, 8)
	if _, err := io.ReadFull(reader, length); err != nil {
		return nil, err
	}
	size := binary.BigEndian.Uint64(length)
	if size > 64<<20 {
		return nil, fmt.Errorf("reset frame too large: %d", size)
	}
	return &StalledConversation{conn: tcp, reader: reader, ResetBytes: size}, nil
}

// StalledConversation holds a reset whose payload has not been read yet.
// The creating test owns its connection through registered cleanup.
type StalledConversation struct {
	conn   *net.TCPConn
	reader *bufio.Reader
	// ResetBytes is the unread reset payload size.
	ResetBytes uint64
}

// Closed resumes the reset and reads through queued frames to the server close.
func (s *StalledConversation) Closed(ctx context.Context) (websocket.StatusCode, string, error) {
	stopped := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		// Cancellation ends this page; its test cleanup also closes it safely.
		_ = s.conn.Close()
		close(stopped)
	})
	defer func() {
		if !stop() {
			<-stopped
		}
	}()
	if deadline, ok := ctx.Deadline(); ok {
		if err := s.conn.SetDeadline(deadline); err != nil {
			return 0, "", err
		}
	}
	// Restore a normal window now that this page is reading again.
	if err := s.conn.SetReadBuffer(1 << 20); err != nil {
		return 0, "", err
	}
	if _, err := io.CopyN(io.Discard, s.reader, int64(s.ResetBytes)); err != nil {
		return 0, "", err
	}
	for {
		var header [2]byte
		if _, err := io.ReadFull(s.reader, header[:]); err != nil {
			return 0, "", err
		}
		if header[0]&0x70 != 0 || header[1]&0x80 != 0 {
			return 0, "", fmt.Errorf("invalid server frame header: %x", header)
		}
		length := uint64(header[1] & 0x7f)
		switch length {
		case 126:
			var extended [2]byte
			if _, err := io.ReadFull(s.reader, extended[:]); err != nil {
				return 0, "", err
			}
			length = uint64(binary.BigEndian.Uint16(extended[:]))
		case 127:
			var extended [8]byte
			if _, err := io.ReadFull(s.reader, extended[:]); err != nil {
				return 0, "", err
			}
			length = binary.BigEndian.Uint64(extended[:])
		}
		if length > 64<<20 {
			return 0, "", fmt.Errorf("server frame too large: %d", length)
		}
		if header[0]&0x0f != 8 {
			if _, err := io.CopyN(io.Discard, s.reader, int64(length)); err != nil {
				return 0, "", err
			}
			continue
		}
		if header[0] != 0x88 || length < 2 || length > 125 {
			return 0, "", fmt.Errorf("invalid close frame: %x, length %d", header, length)
		}
		payload := make([]byte, length)
		if _, err := io.ReadFull(s.reader, payload); err != nil {
			return 0, "", err
		}
		// Echo the close with a zero masking key to finish the handshake.
		frame := append([]byte{0x88, 0x80 | byte(length), 0, 0, 0, 0}, payload...)
		if _, err := s.conn.Write(frame); err != nil {
			return 0, "", err
		}
		return websocket.StatusCode(binary.BigEndian.Uint16(payload[:2])), string(payload[2:]), nil
	}
}

// ConversationText constructs the text frame a page sends for one user message.
func ConversationText(id, text string) *framewire.SendFrame {
	return &framewire.SendFrame{MessageID: core.TurnID(id), Content: []framewire.ClientContent{&framewire.TextContent{Text: text}}}
}
