package artifacts

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"time"

	"github.com/klauspost/compress/zstd"
)

// ContentCoding is the HTTP coding used for command executables.
const ContentCoding = "zstd"

// Effort selects compression cost for published and development artifacts.
type Effort int

const (
	// Published favors download size for an executable encoded once.
	Published Effort = iota
	// Development favors fast encoding at each development startup.
	Development
)

// Encode compresses executable bytes. The pure-Go encoder maps zstd's levels
// to its closest supported compression profiles rather than identical bytes.
func Encode(ctx context.Context, data []byte, effort Effort) ([]byte, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	level := 19
	if effort == Development {
		level = 3
	}
	encoder, err := zstd.NewWriter(nil, zstd.WithEncoderLevel(zstd.EncoderLevelFromZstd(level)))
	if err != nil {
		return nil, err
	}
	defer func() { _ = encoder.Close() }() // EncodeAll has no pending streamed output.
	encoded := encoder.EncodeAll(data, nil)
	return encoded, ctx.Err()
}

// Client owns an artifact HTTP transport. Close releases its idle connections.
type Client struct {
	http      *http.Client
	allowHTTP bool
}

// NewClient makes an HTTPS-only client without redirects.
func NewClient() *Client { return newClient(false) }

// NewClientAllowingHTTP permits HTTP when the digest came from a trusted peer.
func NewClientAllowingHTTP() *Client { return newClient(true) }

func newClient(allowHTTP bool) *Client {
	dialer := &net.Dialer{Timeout: 15 * time.Second}
	transport := &http.Transport{
		Proxy: http.ProxyFromEnvironment,
		DialContext: func(ctx context.Context, network, addr string) (net.Conn, error) {
			conn, err := dialer.DialContext(ctx, network, addr)
			if err != nil {
				return nil, err
			}
			return &progressConn{Conn: conn}, nil
		},
		DisableCompression:    true,
		TLSHandshakeTimeout:   15 * time.Second,
		ResponseHeaderTimeout: 60 * time.Second,
		IdleConnTimeout:       90 * time.Second,
	}
	return &Client{
		http: &http.Client{
			Transport:     transport,
			CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
		},
		allowHTTP: allowHTTP,
	}
}

// Close releases connections retained by the client. Active downloads must end first.
func (c *Client) Close() { c.http.CloseIdleConnections() }

// progressConn enforces the artifact's idle-read timeout, not an overall deadline.
type progressConn struct{ net.Conn }

// Read applies the artifact idle timeout before reading.
func (c *progressConn) Read(b []byte) (int, error) {
	if err := c.SetReadDeadline(time.Now().Add(60 * time.Second)); err != nil {
		return 0, err
	}
	return c.Conn.Read(b)
}

// RejectedError reports a non-success HTTP status, including redirects.
type RejectedError struct {
	// Status is the rejected HTTP response status code.
	Status int
}

// Error reports the rejected HTTP status.
func (e *RejectedError) Error() string { return fmt.Sprintf("the server answered %d", e.Status) }

// CodingError reports a content coding the artifact client cannot decode.
type CodingError struct {
	// Coding is the unsupported Content-Encoding header value.
	Coding string
}

// Error reports the unsupported content coding.
func (e *CodingError) Error() string {
	return fmt.Sprintf("unsupported artifact content coding %q", e.Coding)
}

// downloadError hides potentially signed URLs while retaining error identity.
type downloadError struct{ cause error }

// Error reports a download failure without exposing its URL.
func (e *downloadError) Error() string { return "artifact download failed" }

// Unwrap returns the download failure.
func (e *downloadError) Unwrap() error { return e.cause }

func (c *Client) get(ctx context.Context, location string) (*http.Response, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, location, nil)
	if err != nil {
		return nil, &downloadError{err}
	}
	if req.URL.Scheme != "https" && (!c.allowHTTP || req.URL.Scheme != "http") {
		return nil, &downloadError{errors.New("artifact URL requires HTTPS")}
	}
	req.Header.Set("Accept-Encoding", ContentCoding)
	response, err := c.http.Do(req)
	if err != nil {
		var u *url.Error
		if errors.As(err, &u) {
			err = u.Err
		}
		return nil, &downloadError{err}
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		_ = response.Body.Close() // Discarding a rejected response; no writes to flush.
		return nil, &RejectedError{response.StatusCode}
	}
	coding := response.Header.Get("Content-Encoding")
	if coding != "" && coding != ContentCoding {
		_ = response.Body.Close() // Discarding an unsupported response.
		return nil, &CodingError{coding}
	}
	if coding == ContentCoding {
		decoder, err := zstd.NewReader(response.Body, zstd.WithDecoderConcurrency(1))
		if err != nil {
			_ = response.Body.Close() // Discarding a response whose decoder could not start.
			return nil, &downloadError{err}
		}
		response.Body = &decodedBody{decoder: decoder, body: response.Body}
		response.ContentLength = -1
	}
	return response, nil
}

type decodedBody struct {
	decoder *zstd.Decoder
	body    io.ReadCloser
}

// Read returns decoded artifact bytes.
func (b *decodedBody) Read(p []byte) (int, error) { return b.decoder.Read(p) }

// Close releases the decoder and its response body.
func (b *decodedBody) Close() error {
	b.decoder.Close()
	return b.body.Close()
}

// Download verifies decoded bytes as they arrive. On error the caller discards
// output; it also owns unblocking output if writes can wait indefinitely.
func Download(ctx context.Context, client *Client, location string, expected Digest, output io.Writer) error {
	response, err := client.get(ctx, location)
	if err != nil {
		return err
	}
	defer func() { _ = response.Body.Close() }() // Closing a read body only releases the connection.
	if response.ContentLength >= 0 && uint64(response.ContentLength) != expected.Size {
		return &SizeError{expected.Size, uint64(response.ContentLength)}
	}
	return Copy(ctx, response.Body, expected, output)
}

// DownloadMeasured establishes a digest for a release within limit bytes.
func DownloadMeasured(
	ctx context.Context,
	client *Client,
	location string,
	limit uint64,
	output io.Writer,
) (Digest, error) {
	response, err := client.get(ctx, location)
	if err != nil {
		return Digest{}, err
	}
	defer func() { _ = response.Body.Close() }() // Read-only response ownership ends here.
	if response.ContentLength >= 0 {
		if uint64(response.ContentLength) > limit {
			return Digest{}, &TooLargeError{limit}
		}
		limit = uint64(response.ContentLength)
	}
	m := newMeasure(limit)
	if err := transfer(ctx, response.Body, output, m.update); err != nil {
		return Digest{}, err
	}
	found := m.finish()
	if response.ContentLength >= 0 && found.Size != uint64(response.ContentLength) {
		return Digest{}, &SizeError{uint64(response.ContentLength), found.Size}
	}
	return found, nil
}
