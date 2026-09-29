package commandservice

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"sync"
)

// A Client is the caller's end of one connection to a service: the runner's end
// of the command wire. It is safe for concurrent use.
type Client struct {
	conn *http.ClientConn
}

// Connect binds a client to conn, which the caller has connected to a service:
// HTTP/2 without TLS, with the client speaking first. It takes ownership of
// conn, which [Client.Close] closes.
func Connect(ctx context.Context, conn net.Conn) (*Client, error) {
	transport := &http.Transport{
		DialContext: func(context.Context, string, string) (net.Conn, error) {
			return conn, nil
		},
		Protocols: unencryptedHTTP2(),
		HTTP2: &http.HTTP2Config{
			MaxReceiveBufferPerStream: MaxRecordBytes,
			// The transport adds the window every connection starts with to
			// this one, which together may not pass the largest window.
			MaxReceiveBufferPerConnection: maxHTTP2Window - http2InitialWindow,
		},
		DisableCompression:     true,
		MaxResponseHeaderBytes: maxHeaderBytes,
	}
	connection, err := transport.NewClientConn(ctx, "http", "demi:80")
	if err != nil {
		// The connection is given up; the error says why.
		_ = conn.Close()
		return nil, err
	}
	return &Client{conn: connection}, nil
}

// Close closes the connection, and with it every stream still open.
func (c *Client) Close() error {
	return c.conn.Close()
}

// send sends a request whose URL is a path of the wire.
func (c *Client) send(ctx context.Context, method, path string, body io.ReadCloser, length int64) (*http.Response, error) {
	request, err := http.NewRequestWithContext(ctx, method, "http://demi"+path, nil)
	if err != nil {
		return nil, err
	}
	if body != nil {
		request.Body = body
		request.ContentLength = length
	}
	response, err := c.conn.RoundTrip(request)
	if err != nil {
		return nil, err
	}
	if response.StatusCode < 200 || response.StatusCode > 299 {
		// The body of a refusal carries nothing.
		_ = response.Body.Close()
		return nil, &RejectedError{Status: response.StatusCode}
	}
	return response, nil
}

// Info asks the service for its catalog, which the caller checks against the
// package's descriptor ([PackageDescriptor.Serves]).
func (c *Client) Info(ctx context.Context) (ServiceInfo, error) {
	response, err := c.send(ctx, http.MethodGet, InfoPath, nil, 0)
	if err != nil {
		return ServiceInfo{}, err
	}
	// The catalog is read whole below; closing the body afterwards reports
	// nothing that matters.
	defer response.Body.Close()
	document, err := io.ReadAll(io.LimitReader(response.Body, MaxMetadataBytes+1))
	if err != nil {
		return ServiceInfo{}, err
	}
	return Decode[ServiceInfo](document)
}

// Shutdown asks the service to stop admission and drain: it answers every new
// request 503 while the calls running finish, and then closes the connection.
func (c *Client) Shutdown(ctx context.Context) error {
	response, err := c.send(ctx, http.MethodPost, ShutdownPath, nil, 0)
	if err != nil {
		return err
	}
	return response.Body.Close()
}

// Invoke starts an invocation and returns its stream once the service has
// accepted it. It refuses an invocation that breaks the wire's rules before
// sending it, and returns a [*RejectedError] when the service refuses it: 400
// for invalid metadata, 404 for an unknown operation, 503 for a service that is
// shutting down. ctx bounds the whole invocation: cancelling it cancels the
// stream.
func (c *Client) Invoke(ctx context.Context, invocation Invocation) (*Stream, error) {
	document, err := Encode(invocation)
	if err != nil {
		return nil, err
	}
	return c.open(ctx, InvokePath, document, true)
}

// Conversation sends a request to the conversation endpoint and returns its
// stream. The metadata is the whole request, so it leaves together with the end
// of the request stream: nothing is left to send once the service can answer
// it.
func (c *Client) Conversation(ctx context.Context, request ConversationRequest) (*Stream, error) {
	document, err := Encode(request)
	if err != nil {
		return nil, err
	}
	return c.open(ctx, ConversationPath, document, false)
}

// Numbers opens the service's numbers stream, which the caller answers. The
// service accepts one: a second is refused with a [*RejectedError] of status
// 409.
func (c *Client) Numbers(ctx context.Context) (*NumbersStream, error) {
	document, err := Encode(NumbersOpen{})
	if err != nil {
		return nil, err
	}
	stream, err := c.open(ctx, NumbersPath, document, true)
	if err != nil {
		return nil, err
	}
	return &NumbersStream{stream: stream}, nil
}

// open starts a stream: it sends the metadata, and keeps the request open for
// input when input is true. It returns when the service has answered with its
// response headers.
func (c *Client) open(ctx context.Context, path string, metadata []byte, input bool) (*Stream, error) {
	framed, err := frame(metadata, MaxMetadataBytes)
	if err != nil {
		return nil, err
	}
	requestCtx, cancel := context.WithCancel(ctx)
	stream := &Stream{cancel: cancel}
	var body io.ReadCloser
	length := int64(len(framed))
	if input {
		reader, writer := io.Pipe()
		stream.writer = writer
		body = struct {
			io.Reader
			io.Closer
		}{io.MultiReader(bytes.NewReader(framed), reader), reader}
		length = 0
	} else {
		body = io.NopCloser(bytes.NewReader(framed))
	}
	response, err := c.send(requestCtx, http.MethodPost, path, body, length)
	if err != nil {
		cancel()
		return nil, err
	}
	stream.body = response.Body
	stream.records = NewRecordReader(response.Body)
	// The transport notices a cancelled context only between its writes of the
	// request body, and while the body waits for input it writes nothing, so
	// the stream cancels itself when the caller's context ends.
	stream.watch(context.AfterFunc(ctx, stream.Cancel))
	return stream, nil
}

// A Stream is one open request: its input half and its records. Reading its
// records and writing its input may go on from different goroutines. The
// caller ends a stream by reading it until Next fails ([io.EOF] after a
// completed response) or by calling Cancel; either releases what the stream
// holds.
type Stream struct {
	writer  *io.PipeWriter
	cancel  context.CancelFunc
	body    io.ReadCloser
	records *RecordReader

	// mu guards cancelled and stopWatching.
	mu        sync.Mutex
	cancelled bool
	// stopWatching ends the stream's watch of the caller's context.
	stopWatching func() bool

	release sync.Once
}

// watch keeps the function that ends the stream's watch of the caller's
// context, which [Stream.Cancel] may already have run.
func (s *Stream) watch(stop func() bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.stopWatching = stop
}

// unwatch ends the stream's watch of the caller's context, which nothing is
// left to cancel once the stream is over.
func (s *Stream) unwatch() {
	s.mu.Lock()
	stop := s.stopWatching
	s.mu.Unlock()
	if stop != nil {
		stop()
	}
}

// Write sends one input chunk, at most [MaxRecordBytes]. The service pulls
// input one chunk at a time, and the caller answers each pull with one Write, or
// with End.
//
// A service that answered without reading all its input resets the request
// once its response is complete (RST_STREAM with NO_ERROR), and input sent after
// that, a chunk or its end, is dropped and is not a failure. net/http does not
// tell the code of a reset, so input that the transport stopped taking is
// always dropped, and always goes with a stream that has ended: Next reports how
// it ended, with the completion when the service answered early and with the
// failure otherwise.
func (s *Stream) Write(chunk []byte) error {
	framed, err := EncodeInput(chunk)
	if err != nil {
		return err
	}
	if s.writer == nil {
		return errors.New("the request carries no input")
	}
	_, err = s.writer.Write(framed)
	return s.unlessAnswered(err)
}

// End ends the input: the end of the request stream. Like [Stream.Write], it
// drops what the transport no longer takes.
func (s *Stream) End() error {
	if s.writer == nil {
		return nil
	}
	return s.unlessAnswered(s.writer.Close())
}

// unlessAnswered turns the failure of an input write to a stream that is over
// into success. The transport closes the request body once the stream is over,
// which is all it tells.
func (s *Stream) unlessAnswered(err error) error {
	if err == nil {
		return nil
	}
	if s.isCancelled() {
		return ErrCancelled
	}
	if errors.Is(err, io.ErrClosedPipe) {
		return nil
	}
	return err
}

func (s *Stream) isCancelled() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.cancelled
}

// Cancel resets the stream with RST_STREAM(CANCEL). The service cancels the
// invocation, and Next fails with [ErrCancelled].
func (s *Stream) Cancel() {
	s.mu.Lock()
	first := !s.cancelled
	s.cancelled = true
	s.mu.Unlock()
	if !first {
		return
	}
	if s.writer != nil {
		// The transport waits for input where it would see the cancellation,
		// so the wait ends with an error, which the transport answers with the
		// reset. CloseWithError always returns nil.
		s.writer.CloseWithError(ErrCancelled)
	}
	s.cancel()
	s.unwatch()
}

// Next returns the next record of the response. After the completion record it
// returns [io.EOF]. It returns [ErrIncomplete] when the response ends without a
// completion, [ErrCancelled] when the stream was cancelled, and the other
// errors of [RecordReader.Next].
func (s *Stream) Next() (Record, error) {
	record, err := s.records.Next()
	if err != nil {
		s.finish()
		if s.isCancelled() || errors.Is(err, context.Canceled) {
			return Record{}, ErrCancelled
		}
		return Record{}, err
	}
	return record, nil
}

// finish releases what the stream holds once its response is over.
func (s *Stream) finish() {
	s.release.Do(func() {
		s.unwatch()
		s.cancel()
		// The body's close error says only that the stream was over already.
		_ = s.body.Close()
	})
}
