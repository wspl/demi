package process

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runnerwire"
)

const pipeAnswerBytes = 16 * 1024

// PipeClient carries file contents and output through backend pipe routes.
// Its owner calls Close after cancelling and joining all pipe operations.
type PipeClient struct {
	http      *http.Client
	transport *http.Transport
	origin    *url.URL
	token     func() (runnerwire.DeviceToken, bool)
}

// NewPipeClient uses a 15-second connection timeout. Token returns the current
// registration credential and its presence; it must be safe for concurrent calls.
func NewPipeClient(backend runnerwire.BackendURL, token func() (runnerwire.DeviceToken, bool)) (*PipeClient, error) {
	return NewPipeClientWithConnectTimeout(backend, token, 15*time.Second)
}

// NewPipeClientWithConnectTimeout bounds connection opening only. Requests,
// answers and bodies may remain quiet for as long as their contexts allow.
func NewPipeClientWithConnectTimeout(backend runnerwire.BackendURL, token func() (runnerwire.DeviceToken, bool), timeout time.Duration) (*PipeClient, error) {
	origin, err := url.Parse(backend.String())
	if err != nil {
		return nil, err
	}
	switch origin.Scheme {
	case "ws":
		origin.Scheme = "http"
	case "wss":
		origin.Scheme = "https"
	}
	origin.Path = "/"
	origin.RawPath = ""
	origin.RawQuery = ""
	origin.ForceQuery = false
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		opening, cancel := context.WithTimeout(ctx, timeout)
		defer cancel()
		return (&net.Dialer{}).DialContext(opening, network, address)
	}
	transport.DialTLSContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		opening, cancel := context.WithTimeout(ctx, timeout)
		defer cancel()
		connection, err := (&net.Dialer{}).DialContext(opening, network, address)
		if err != nil {
			return nil, err
		}
		host, _, err := net.SplitHostPort(address)
		if err != nil {
			return nil, errors.Join(err, connection.Close())
		}
		secured := tls.Client(connection, &tls.Config{ServerName: host, MinVersion: tls.VersionTLS12})
		if err := secured.HandshakeContext(opening); err != nil {
			return nil, errors.Join(err, connection.Close())
		}
		return secured, nil
	}
	transport.TLSHandshakeTimeout = timeout
	transport.ResponseHeaderTimeout = 0
	client := &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	return &PipeClient{http: client, transport: transport, origin: origin, token: token}, nil
}

// Open reads an origin-relative pipe route. The caller closes the body; its
// context remains active for the body's entire lifetime.
func (c *PipeClient) Open(ctx context.Context, path string) (io.ReadCloser, error) {
	response, err := cmdsdk.Retry(ctx, func() (*http.Response, error) {
		request, err := c.request(ctx, http.MethodGet, path, nil)
		if err != nil {
			return nil, err
		}
		return c.http.Do(request)
	})
	if err != nil {
		if ctx.Err() != nil {
			return nil, &operationFailure{message: "pipe cancelled", cause: ctx.Err()}
		}
		return nil, fmt.Errorf("pipe: %w", err)
	}
	if err := pipeOK(response); err != nil {
		return nil, err
	}
	return response.Body, nil
}

// Put sends an origin-relative pipe route and reads its bounded confirmation.
// It takes ownership of body and closes it on every outcome. Close must unblock Read.
func (c *PipeClient) Put(ctx context.Context, path string, body io.ReadCloser) (err error) {
	defer func() {
		if err != nil && ctx.Err() != nil {
			err = &operationFailure{message: "pipe cancelled", cause: ctx.Err()}
		}
	}()
	upload := &pipeUpload{body: body}
	defer upload.close()
	stopped := interruptCommandIO(ctx, upload)
	defer stopped()
	var backoff cmdsdk.Backoff
	for {
		attempt := &pipeAttempt{upload: upload, closed: make(chan struct{})}
		request, err := c.request(ctx, http.MethodPut, path, attempt)
		if err != nil {
			return err
		}
		response, err := c.http.Do(request)
		if response != nil {
			upload.close()
		} // An early response ends the upload, including a blocked input read.
		// net/http may close request bodies asynchronously. Join that ownership
		// before retrying or returning to the caller.
		<-attempt.closed
		if err != nil {
			if !upload.read.Load() && cmdsdk.Exhausted(err) {
				if err := backoff.Wait(ctx); err != nil {
					return err
				}
				continue
			}
			return fmt.Errorf("pipe: %w", err)
		}
		if err := pipeOK(response); err != nil {
			return err
		}
		defer func() { _ = response.Body.Close() }() // Read errors below carry any transport failure.
		answer, err := io.ReadAll(io.LimitReader(response.Body, pipeAnswerBytes+1))
		if err != nil {
			return fmt.Errorf("pipe confirmation: %w", err)
		}
		if len(answer) > pipeAnswerBytes {
			return fmt.Errorf("oversized pipe confirmation")
		}
		return nil
	}
}

// pipeUpload keeps an unread body intact across descriptor-exhausted attempts.
type pipeUpload struct {
	body io.ReadCloser
	read atomic.Bool
	once sync.Once
}

func (u *pipeUpload) close() {
	u.once.Do(func() { _ = u.body.Close() }) // Completion/cancellation already supplies the outcome.
}
func (u *pipeUpload) Close() error {
	u.close()
	return nil
}

type pipeAttempt struct {
	upload *pipeUpload
	closed chan struct{}
	once   sync.Once
}

func (a *pipeAttempt) Read(b []byte) (int, error) {
	a.upload.read.Store(true)
	return a.upload.body.Read(b)
}
func (a *pipeAttempt) Close() error {
	a.once.Do(func() {
		if a.upload.read.Load() {
			a.upload.close()
		}
		close(a.closed)
	})
	return nil
}

// request confines authenticated pipe traffic to its backend origin.
func (c *PipeClient) request(ctx context.Context, method, path string, body io.Reader) (*http.Request, error) {
	if !strings.HasPrefix(path, "/") || strings.HasPrefix(path, "//") || strings.Contains(path, "\\") {
		return nil, fmt.Errorf("pipe URL must be origin-relative")
	}
	relative, err := url.Parse(path)
	if err != nil {
		return nil, err
	}
	target := c.origin.ResolveReference(relative)
	if target.Scheme != c.origin.Scheme || target.Host != c.origin.Host {
		return nil, fmt.Errorf("pipe URL changed backend origin")
	}
	token, ok := c.token()
	if !ok {
		return nil, fmt.Errorf("runner has no device token")
	}
	request, err := http.NewRequestWithContext(ctx, method, target.String(), body)
	if err != nil {
		return nil, err
	}
	request.Header.Set("Authorization", "Bearer "+token.Expose())
	return request, nil
}

// pipeOK consumes a bounded refusal; successful response bodies stay owned by the caller.
func pipeOK(response *http.Response) error {
	if response.StatusCode == http.StatusOK {
		return nil
	}
	defer func() { _ = response.Body.Close() }() // Cleanup follows the operation result; cancellation may already have closed it.
	body, err := io.ReadAll(io.LimitReader(response.Body, pipeAnswerBytes))
	if err != nil {
		return fmt.Errorf("pipe refusal: %w", err)
	}
	return fmt.Errorf("pipe refused (%s): %s", response.Status, streamText(body))
}

// Close releases idle transport connections after operations have finished.
func (c *PipeClient) Close() error {
	c.transport.CloseIdleConnections()
	return nil
}

// ReportPipe encodes and sends a pipe outcome; cancellation ends the wait when
// the backend is gone. Output carries encoded runner wire frames.
func ReportPipe(ctx context.Context, output chan<- []byte, id string, result error) error {
	report := runnerwire.PipeDone{PipeID: id, Ok: result == nil}
	if result != nil {
		message := result.Error()
		report.Error = &message
	}
	frame, err := runnerwire.Encode(&report)
	if err != nil {
		return fmt.Errorf("pipe result encoding failed: %w", err)
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case output <- frame:
		return nil
	}
}
