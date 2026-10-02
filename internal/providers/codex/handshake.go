package codex

import (
	"bytes"
	"io"
	"net/http"
)

// handshakeTransport preserves the vendor's complete refused handshake before
// coder/websocket truncates diagnostic bodies to 1024 bytes. It uses the owner's
// transport, without taking ownership of that transport's pooled connections.
type handshakeTransport struct {
	base    http.RoundTripper
	refusal *refusal
}

func (t *handshakeTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	response, err := t.base.RoundTrip(request)
	if err != nil || response.StatusCode == http.StatusSwitchingProtocols {
		return response, err
	}
	data, readErr := io.ReadAll(response.Body)
	_ = response.Body.Close() // The handshake error retains status and headers even if reading failed.
	if readErr != nil {
		data = nil
	}
	t.refusal = &refusal{status: response.StatusCode, headers: response.Header.Clone(), body: string(bytes.ToValidUTF8(data, []byte("�")))}
	response.Body = io.NopCloser(bytes.NewReader(data))
	return response, nil
}
