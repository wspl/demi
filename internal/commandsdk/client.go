package commandsdk

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"sync"
	"sync/atomic"
	"time"

	"github.com/wspl/demi/internal/commandproto"
)

const (
	phaseTimeout  = 10 * time.Second
	cancelTimeout = 5 * time.Second
)

func protocols() *http.Protocols {
	p := new(http.Protocols)
	p.SetUnencryptedHTTP2(true)
	return p
}

func h2Config(client bool) *http.HTTP2Config {
	window := int(1<<31 - 1)
	if client {
		window -= 65535
	}
	// The handler limit exceeds the number of stream IDs on a connection, so
	// neither admission nor its early-reset queue guard can bind.
	return &http.HTTP2Config{
		MaxConcurrentStreams:          1<<31 - 1,
		MaxReceiveBufferPerConnection: window,
		MaxReceiveBufferPerStream:     commandproto.MaxRecordBytes,
	}
}

// Client owns exactly one HTTP/2 connection. Close releases it.
type Client struct{ connection *http.ClientConn }

// Connect takes ownership of a caller-supplied duplex connection.
func Connect(ctx context.Context, conn net.Conn) (*Client, error) {
	// NewClientConn writes its preface synchronously. Close the owned transport
	// on cancellation so even a peer that reads nothing cannot strand that write.
	interrupted := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		// Cancellation already supplies the failure; closing interrupts the write.
		_ = conn.Close()
		close(interrupted)
	})
	stopInterrupt := sync.OnceFunc(func() {
		if !stop() {
			<-interrupted
		}
	})
	defer stopInterrupt()
	var dialed atomic.Bool
	t := &http.Transport{
		Protocols:              protocols(),
		HTTP2:                  h2Config(true),
		MaxResponseHeaderBytes: 16 * 1024,
		DialContext: func(context.Context, string, string) (net.Conn, error) {
			if dialed.Swap(true) {
				return nil, errors.New("command connection already taken")
			}
			return conn, nil
		},
	}
	cc, err := t.NewClientConn(ctx, "http", "demi:80")
	if err != nil {
		return nil, errors.Join(err, conn.Close())
	}
	stopInterrupt()
	if err := ctx.Err(); err != nil {
		return nil, errors.Join(err, cc.Close())
	}
	return &Client{connection: cc}, nil
}

// Close closes the service connection and cancels its streams.
func (c *Client) Close() error {
	return c.connection.Close()
}

func (c *Client) request(ctx context.Context, method, path string, body io.Reader) (*http.Response, error) {
	req, err := http.NewRequestWithContext(ctx, method, "http://demi"+path, body)
	if err != nil {
		return nil, err
	}
	response, err := c.connection.RoundTrip(req)
	if err != nil {
		return nil, err
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, errors.Join(
			fmt.Errorf("service rejected HTTP request with status %d", response.StatusCode),
			response.Body.Close(),
		)
	}
	return response, nil
}

// Info reads and validates the bounded service catalog using the caller's context.
func (c *Client) Info(ctx context.Context) (commandproto.ServiceInfo, error) {
	r, err := c.request(ctx, http.MethodGet, commandproto.InfoPath, nil)
	if err != nil {
		return commandproto.ServiceInfo{}, err
	}
	defer func() {
		_ = r.Body.Close()
	}() // Reading the bounded response reports transport failures.
	b, err := io.ReadAll(io.LimitReader(r.Body, commandproto.MaxMetadataBytes+1))
	if err != nil {
		return commandproto.ServiceInfo{}, err
	}
	if len(b) > commandproto.MaxMetadataBytes {
		return commandproto.ServiceInfo{}, commandproto.ErrTooLarge
	}
	return commandproto.DecodeServiceInfo(b)
}

// Invoke opens one invocation. The caller must drain output or cancel input.
func (c *Client) Invoke(ctx context.Context, m commandproto.Metadata) (*CommandInput, *CommandOutput, error) {
	b, err := commandproto.EncodeMetadata(m)
	if err != nil {
		return nil, nil, err
	}
	return c.invokeAt(ctx, commandproto.InvokePath, b, false)
}

// Conversation sends the complete metadata-only lifecycle request with EOF.
func (c *Client) Conversation(
	ctx context.Context,
	m commandproto.ConversationRequest,
) (*CommandInput, *CommandOutput, error) {
	b, err := commandproto.EncodeConversationRequest(m)
	if err != nil {
		return nil, nil, err
	}
	return c.invokeAt(ctx, commandproto.ConversationPath, b, true)
}

func (c *Client) invokeAt(
	ctx context.Context,
	path string,
	b []byte,
	finite bool,
) (*CommandInput, *CommandOutput, error) {
	ctx, cancel := context.WithCancel(ctx)
	reader, writer := io.Pipe()
	var body io.Reader = &requestBody{Reader: io.MultiReader(bytes.NewReader(b), reader), closer: reader}
	if finite {
		body = bytes.NewReader(b)
	}
	r, err := c.request(ctx, http.MethodPost, path, body)
	if err != nil {
		cancel()
		return nil, nil, errors.Join(err, reader.Close(), writer.Close())
	}
	state := &invocationState{cancel: cancel, reader: reader, writer: writer}
	return &CommandInput{state: state}, &CommandOutput{body: r.Body, state: state}, nil
}

// Shutdown stops admission and asks the service to drain.
func (c *Client) Shutdown(ctx context.Context) error {
	r, err := c.request(ctx, http.MethodPost, commandproto.ShutdownPath, nil)
	if err != nil {
		return err
	}
	return r.Body.Close()
}

type invocationState struct {
	cancel    context.CancelFunc
	reader    *io.PipeReader
	writer    *io.PipeWriter
	completed atomic.Bool
	once      sync.Once
}

func (s *invocationState) close() {
	s.once.Do(func() {
		s.cancel()
		_ = s.reader.Close()
		_ = s.writer.Close()
	})
}

// CommandInput writes requested chunks or cancels one invocation.
type CommandInput struct{ state *invocationState }

// Write sends exactly one bounded chunk. A completed peer discards later input.
func (i *CommandInput) Write(ctx context.Context, b []byte) error {
	encoded, err := commandproto.EncodeInput(b)
	if err != nil {
		return err
	}
	stop := context.AfterFunc(ctx, i.state.close)
	defer stop()
	_, err = i.state.writer.Write(encoded)
	if i.state.completed.Load() {
		return nil
	}
	return err
}

// End signals input EOF without ending the invocation.
func (i *CommandInput) End() error {
	return i.state.writer.Close()
}

// Cancel resets this invocation without closing the service connection.
func (i *CommandInput) Cancel() {
	i.state.close()
}

// CommandOutput decodes bounded records and requires exactly one final completion.
type CommandOutput struct {
	body    io.ReadCloser
	state   *invocationState
	decoder commandproto.RecordDecoder
	pending []byte
	ended   bool
}

// Next reads one response record; io.EOF means validated completion and stream end.
func (o *CommandOutput) Next(ctx context.Context) (commandproto.Record, error) {
	if o.ended {
		return nil, io.EOF
	}
	stop := context.AfterFunc(ctx, o.state.close)
	defer stop()
	for {
		if len(o.pending) > 0 {
			r, n, err := o.decoder.Decode(o.pending)
			o.pending = o.pending[n:]
			if err != nil {
				o.state.close()
				return nil, err
			}
			if r != nil {
				if _, ok := r.(commandproto.Completed); ok {
					o.state.completed.Store(true)
					_ = o.state.writer.Close()
				}
				return r, nil
			}
		}
		b := make([]byte, commandproto.MaxRecordBytes)
		n, err := o.body.Read(b)
		if n > 0 {
			o.pending = b[:n]
			continue
		}
		if err != nil {
			o.ended = true
			closeErr := o.body.Close()
			o.state.close()
			if errors.Is(err, io.EOF) {
				if e := o.decoder.Finish(); e != nil {
					return nil, e
				}
				return nil, errors.Join(io.EOF, closeErr)
			}
			return nil, errors.Join(err, closeErr)
		}
	}
}

// requestBody keeps the input pipe interruptible when HTTP/2 closes a stream.
type requestBody struct {
	io.Reader
	closer io.Closer
}

// Close releases the owned transport.
func (b *requestBody) Close() error {
	return b.closer.Close()
}
