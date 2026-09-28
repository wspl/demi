package commandservice

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"sync"
)

// Client owns one caller-side command-service connection.
type Client struct{ conn *http.ClientConn }

// Connect binds the client to conn using unencrypted HTTP/2, with a ten-second handshake limit.
func Connect(ctx context.Context, conn net.Conn) (*Client, error) {
	dialed := false
	config := http2Config()
	// Go 1.27 adds this client setting to the initial HTTP/2 window.
	config.MaxReceiveBufferPerConnection -= 65535
	t := &http.Transport{
		Protocols:              protocols(),
		HTTP2:                  config,
		MaxResponseHeaderBytes: HeaderListBytes - httpHeaderAdjustment,
		DialContext: func(context.Context, string, string) (net.Conn, error) {
			if dialed {
				return nil, fmt.Errorf("connection already used")
			}
			dialed = true
			return conn, nil
		},
	}
	handshake, cancel := context.WithTimeout(ctx, phaseTimeout)
	defer cancel()
	// The supplied connection can block while the transport writes its preface.
	stop := context.AfterFunc(handshake, func() {
		// Closing interrupts handshake IO; the handshake error is returned to the caller.
		_ = conn.Close()
	})
	defer stop()
	c, err := t.NewClientConn(handshake, "http", "demi:80")
	if err != nil {
		return nil, err
	}
	return &Client{conn: c}, nil
}

func (c *Client) request(ctx context.Context, method, path string, body io.Reader) (*http.Response, error) {
	r, err := http.NewRequestWithContext(ctx, method, "http://demi"+path, body)
	if err != nil {
		return nil, err
	}
	response, err := c.conn.RoundTrip(r)
	if err != nil {
		return nil, err
	}
	if response.StatusCode < http.StatusOK || response.StatusCode >= http.StatusMultipleChoices {
		// Rejection owns the outcome; draining or closing this body adds no useful error.
		_ = response.Body.Close()
		return nil, &RejectedError{Status: response.StatusCode}
	}
	return response, nil
}

// Info reads and validates the service catalog within ten seconds.
func (c *Client) Info(ctx context.Context) (ServiceInfo, error) {
	ctx, cancel := context.WithTimeout(ctx, phaseTimeout)
	defer cancel()
	r, err := c.request(ctx, http.MethodGet, InfoPath, nil)
	if err != nil {
		return ServiceInfo{}, err
	}
	// ReadAll below reports response IO failures; Close only releases its resources.
	defer r.Body.Close()
	b, err := io.ReadAll(io.LimitReader(r.Body, MaxMetadataBytes+1))
	if err != nil {
		return ServiceInfo{}, err
	}
	if len(b) > MaxMetadataBytes {
		return ServiceInfo{}, ErrTooLarge
	}
	return Decode[ServiceInfo](b)
}

// Close closes the client connection and interrupts its streams.
func (c *Client) Close() error { return c.conn.Close() }

// Shutdown requests service admission to stop and existing calls to drain.
func (c *Client) Shutdown(ctx context.Context) error {
	r, err := c.request(ctx, http.MethodPost, ShutdownPath, nil)
	if err != nil {
		return err
	}
	return r.Body.Close()
}

// Invoke opens an invocation and leaves its input available until End or cancellation.
func (c *Client) Invoke(ctx context.Context, v Invocation) (*Stream, error) {
	b, err := EncodeMetadata(v)
	if err != nil {
		return nil, err
	}
	return c.open(ctx, InvokePath, b, false)
}

// Conversation sends finite lifecycle metadata and returns the response stream.
func (c *Client) Conversation(ctx context.Context, v ConversationRequest) (*Stream, error) {
	b, err := EncodeMetadata(v)
	if err != nil {
		return nil, err
	}
	return c.open(ctx, ConversationPath, b, true)
}

// Numbers opens the service’s numbers stream for the caller to answer.
func (c *Client) Numbers(ctx context.Context) (*NumbersStream, error) {
	b, err := EncodeMetadata(NumbersOpen{})
	if err != nil {
		return nil, err
	}
	s, err := c.open(ctx, NumbersPath, b, false)
	if err != nil {
		return nil, err
	}
	return &NumbersStream{stream: s}, nil
}

func (c *Client) open(ctx context.Context, path string, b []byte, finite bool) (*Stream, error) {
	ctx, cancel := context.WithCancel(ctx)
	reader, writer := io.Pipe()
	var body io.Reader = &requestBody{Reader: io.MultiReader(bytes.NewReader(b), reader), Closer: reader}
	if finite {
		body = bytes.NewReader(b)
		// The finite request has no pipe input; these in-memory closes always return nil.
		_ = writer.Close()
		_ = reader.Close()
	}
	r, err := c.request(ctx, http.MethodPost, path, body)
	if err != nil {
		cancel()
		// Pipe close publishes the original request error to any waiting input operation.
		_ = writer.CloseWithError(err)
		_ = reader.CloseWithError(err)
		return nil, err
	}
	return &Stream{input: writer, body: r.Body, decoder: RecordDecoder{Reader: r.Body}, cancel: cancel}, nil
}

// requestBody lets the transport interrupt a pending pipe read on reset.
type requestBody struct {
	io.Reader
	io.Closer
}

// Stream is one request’s bounded input and response records. Next has one reader; Write
// and End supply its input.
type Stream struct {
	input   *io.PipeWriter
	body    io.ReadCloser
	decoder RecordDecoder
	cancel  context.CancelFunc
	ended   bool
	mu      sync.Mutex
}

// Write sends one bounded input chunk. If the transport stopped accepting
// input, the chunk is dropped; Next reports whether the stream succeeded.
func (s *Stream) Write(b []byte) error {
	framed, err := EncodeInput(b)
	if err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	_, err = s.input.Write(framed)
	if errors.Is(err, io.ErrClosedPipe) || errors.Is(err, ErrCancelled) {
		return nil
	}
	return err
}

// End signals input EOF. A transport that already stopped taking input needs no further action.
func (s *Stream) End() error { return s.input.Close() }

// Cancel resets the request and interrupts its input and response IO.
func (s *Stream) Cancel() {
	s.cancel()
	// Cancellation owns the outcome; pipe close is infallible and body close is teardown.
	_ = s.input.CloseWithError(ErrCancelled)
	_ = s.body.Close()
}

// Next reads the next response record and continues returning io.EOF after its end.
func (s *Stream) Next() (Record, error) {
	if s.ended {
		return Record{}, io.EOF
	}
	record, err := s.decoder.Next()
	if err != nil {
		if err == io.EOF {
			s.ended = true
		}
		s.Cancel()
	}
	return record, err
}
