package artifact

import (
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptrace"
	"net/url"
	"time"

	// The roots of Mozilla's bundle, which crypto/x509 uses only when the system
	// has none.
	_ "golang.org/x/crypto/x509roots/fallback"
)

// The limits of a download's connection.
const (
	// connectTimeout is how long a connection may take to be ready, from the
	// start of the request: the dial, a proxy's CONNECT and the TLS handshake
	// share it, as one connect timeout covers them in the Rust's client.
	connectTimeout = 15 * time.Second
	// readTimeout is how long the server may take to answer, and how long a
	// read of its body may wait for bytes.
	readTimeout = 60 * time.Second
)

// errReadTimeout is why a read of a body was given up.
var errReadTimeout = errors.New("timed out waiting for the server")

// errConnectTimeout is why a connection was given up.
var errConnectTimeout = errors.New("timed out connecting to the server")

// NewClient returns the client every artifact download uses: HTTPS only, no
// redirects, 15 seconds to connect (a proxy's CONNECT and the TLS handshake
// included) and 60 seconds for any read. It trusts the system's certificate roots ($SSL_CERT_FILE on
// Linux), or Mozilla's bundle on a system that has none, and reads the proxy
// settings of the environment.
func NewClient() *http.Client {
	return newClient(false, http.ProxyFromEnvironment, (&net.Dialer{}).DialContext)
}

// NewClientAllowingHTTP is [NewClient], but plain HTTP too: for a caller that
// received the declared size and SHA-256 over a connection it trusts, so the
// transport cannot change what it keeps, such as a runner installing a command
// executable (docs/execution/native-runtime.md § Install the selected
// executable). A download whose digest comes from the same server keeps
// [NewClient].
func NewClientAllowingHTTP() *http.Client {
	return newClient(true, http.ProxyFromEnvironment, (&net.Dialer{}).DialContext)
}

// A dialFunc makes the connection of a request.
type dialFunc func(ctx context.Context, network, address string) (net.Conn, error)

// newClient makes a client that reaches servers through proxy, which may be nil,
// and makes its connections with dial.
func newClient(allowHTTP bool, proxy func(*http.Request) (*url.URL, error), dial dialFunc) *http.Client {
	var transport http.RoundTripper = &http.Transport{
		Proxy: proxy,
		// A connection is ready within one budget, however it is made (get). The
		// transport goes on making a connection whose request gave up, for the
		// next request; these limits only end that work, and no step of a
		// connection that is within the budget reaches them.
		DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
			ctx, cancel := context.WithTimeout(ctx, connectTimeout)
			defer cancel()
			return dial(ctx, network, address)
		},
		TLSHandshakeTimeout:   connectTimeout,
		ForceAttemptHTTP2:     true,
		ResponseHeaderTimeout: readTimeout,
		// A body is counted as it arrives, so it must arrive as it was sent.
		DisableCompression: true,
	}
	if !allowHTTP {
		transport = httpsOnly{transport}
	}
	return &http.Client{
		Transport: transport,
		// With redirects off, a redirect is an answer that is not the artifact.
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
}

// httpsOnly refuses a request that is not HTTPS.
type httpsOnly struct {
	next http.RoundTripper
}

func (h httpsOnly) RoundTrip(request *http.Request) (*http.Response, error) {
	if request.URL.Scheme != "https" {
		if request.Body != nil {
			request.Body.Close()
		}
		return nil, errors.New("URL scheme is not allowed")
	}
	return h.next.RoundTrip(request)
}

// Download streams url into output, enforcing the declared size and SHA-256.
// The caller owns output and discards it on any failure.
func Download(ctx context.Context, client *http.Client, url string, expected Digest, output io.Writer) error {
	response, err := get(ctx, client, url)
	if err != nil {
		return err
	}
	defer response.close()
	if length := response.ContentLength; length >= 0 && uint64(length) != expected.Size {
		return &SizeError{Declared: expected.Size, Actual: uint64(length)}
	}
	verifier := NewVerifier(expected)
	if err := response.copy(ctx, io.MultiWriter(verifier, output)); err != nil {
		return err
	}
	return verifier.Finish()
}

// DownloadMeasured streams url into output and returns the size and SHA-256 of
// what arrived, for bytes nobody has declared a digest for yet, such as an
// archive whose release record is being prepared. More than limit bytes fail,
// and so does a body other than the length the server declared. The caller owns
// output and discards it on any failure.
func DownloadMeasured(ctx context.Context, client *http.Client, url string, limit uint64, output io.Writer) (Digest, error) {
	response, err := get(ctx, client, url)
	if err != nil {
		return Digest{}, err
	}
	defer response.close()
	declared := int64(-1)
	if response.ContentLength >= 0 {
		declared = response.ContentLength
		if uint64(declared) > limit {
			return Digest{}, &TooLargeError{Declared: limit}
		}
		// Bytes past a declared length fail as soon as they arrive.
		limit = uint64(declared)
	}
	m := newMeasure(limit)
	if err := response.copy(ctx, io.MultiWriter(m, output)); err != nil {
		return Digest{}, err
	}
	measured := m.digest()
	if declared >= 0 && uint64(declared) != measured.Size {
		return Digest{}, &SizeError{Declared: uint64(declared), Actual: measured.Size}
	}
	return measured, nil
}

// Copy copies input, such as a local file, into output, enforcing the declared
// size and SHA-256.
func Copy(ctx context.Context, input io.Reader, expected Digest, output io.Writer) error {
	verifier := NewVerifier(expected)
	if err := copyChunks(ctx, input, io.MultiWriter(verifier, output), nil); err != nil {
		return err
	}
	return verifier.Finish()
}

// A response is the answer to a request that succeeded, and what waiting for
// its body needs.
type response struct {
	*http.Response
	// cancel ends the request, and with it the read of the body.
	cancel context.CancelCauseFunc
}

// get sends a GET of url and returns its answer once it is a success. The
// message of a failure leaves out the URL, which may carry a signature.
func get(ctx context.Context, client *http.Client, url string) (*response, error) {
	requestCtx, cancel := context.WithCancelCause(ctx)
	// The connect budget runs from here until the connection is ready.
	connecting := time.AfterFunc(connectTimeout, func() { cancel(errConnectTimeout) })
	defer connecting.Stop()
	ready := &httptrace.ClientTrace{GotConn: func(httptrace.GotConnInfo) { connecting.Stop() }}
	request, err := http.NewRequestWithContext(httptrace.WithClientTrace(requestCtx, ready), http.MethodGet, url, nil)
	if err != nil {
		cancel(nil)
		return nil, failed(ctx, requestCtx, err)
	}
	answer, err := client.Do(request)
	if err != nil {
		cancel(nil)
		return nil, failed(ctx, requestCtx, err)
	}
	if answer.StatusCode < 200 || answer.StatusCode > 299 {
		answer.Body.Close()
		cancel(nil)
		return nil, &RejectedError{Status: answer.StatusCode}
	}
	return &response{Response: answer, cancel: cancel}, nil
}

// close releases the response.
func (r *response) close() {
	r.Body.Close()
	r.cancel(nil)
}

// copy writes the body into output, chunk by chunk. A read that waits longer
// than the read timeout for bytes fails.
func (r *response) copy(ctx context.Context, output io.Writer) error {
	idle := time.AfterFunc(readTimeout, func() { r.cancel(errReadTimeout) })
	defer idle.Stop()
	body := readFunc(func(p []byte) (int, error) {
		idle.Reset(readTimeout)
		return r.Body.Read(p)
	})
	return copyChunks(ctx, body, output, func(err error) error {
		if cause := context.Cause(ctx); cause != nil {
			return cause
		}
		if errors.Is(err, context.Canceled) {
			return &DownloadError{Message: errReadTimeout.Error()}
		}
		return &DownloadError{Message: message(err)}
	})
}

// A readFunc is an [io.Reader].
type readFunc func([]byte) (int, error)

func (f readFunc) Read(p []byte) (int, error) { return f(p) }

// failed says what a failure of a request means: the caller's cancellation when
// that is what it was, else a download that failed.
func failed(ctx, requestCtx context.Context, err error) error {
	if cause := context.Cause(ctx); cause != nil {
		return cause
	}
	if cause := context.Cause(requestCtx); errors.Is(cause, errReadTimeout) || errors.Is(cause, errConnectTimeout) {
		return &DownloadError{Message: cause.Error()}
	}
	return &DownloadError{Message: message(err)}
}

// message is the text of a request's failure without its URL.
func message(err error) string {
	var failure *url.Error
	if errors.As(err, &failure) {
		return failure.Err.Error()
	}
	return err.Error()
}
