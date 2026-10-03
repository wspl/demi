package backendtest

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/netip"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/webapi"
)

// TestBackend is a running fixture and its HTTP client. Harness.Start owns
// its cleanup; Close also permits restart over the same durable data.
type TestBackend struct {
	URL     string
	Backend *backend.Backend
	HTTP    *http.Client
}

// Session is a signed-in user's session, as their browser holds it: its cookie
// on every request.
type Session struct {
	Cookie string
	User   webapi.UserDTO
}

// Answer is an HTTP answer, read whole.
type Answer struct {
	Status  int
	Headers http.Header
	Body    []byte
}

// DecodeAnswer decodes an answer using its contract's generated decoder.
// Supplying the decoder keeps validation at entry; no reflection decoder is used.
func DecodeAnswer[T any](answer Answer, decode func([]byte) (T, error)) (T, error) {
	panic("not written: b-backend")
}

// ErrorBody validates and decodes the backend's error contract.
func (a Answer) ErrorBody() (webapi.ErrorBody, error) { panic("not written: b-backend") }

// Refusal returns the answer's HTTP status and validated error code.
func (a Answer) Refusal() (int, webapi.ErrorCode, error) { panic("not written: b-backend") }

// SessionCookies returns Set-Cookie values that set or clear the session cookie.
func (a Answer) SessionCookies() []string { panic("not written: b-backend") }

// ReadAnswer reads the whole response and closes its body on every exit.
func ReadAnswer(ctx context.Context, response *http.Response) (Answer, error) {
	panic("not written: b-backend")
}

// SessionFrom validates a setup or login identity and its one session cookie.
func SessionFrom(answer Answer) (Session, error) { panic("not written: b-backend") }

// Close joins backend shutdown and reports all failed steps.
func (b *TestBackend) Close(ctx context.Context) error { panic("not written: b-backend") }

// Address returns the listener's bound address.
func (b *TestBackend) Address() netip.AddrPort { panic("not written: b-backend") }

// WSURL returns the ws:// URL of path on this backend.
func (b *TestBackend) WSURL(path string) string { panic("not written: b-backend") }

// Send sends a request with an optional cookie and pre-encoded JSON body.
// A nil body sends no JSON. Callers use generated encoders or contract.EncodeJSON.
func (b *TestBackend) Send(ctx context.Context, method, path string, cookie *string, body json.RawMessage) (Answer, error) {
	panic("not written: b-backend")
}

// Read sends GET with the optional browser session cookie.
func (b *TestBackend) Read(ctx context.Context, path string, session *Session) (Answer, error) {
	panic("not written: b-backend")
}

// Post sends POST with the optional session cookie and pre-encoded JSON body.
func (b *TestBackend) Post(ctx context.Context, path string, session *Session, body json.RawMessage) (Answer, error) {
	panic("not written: b-backend")
}

// Patch sends PATCH with the session cookie and pre-encoded JSON body.
func (b *TestBackend) Patch(ctx context.Context, path string, session *Session, body json.RawMessage) (Answer, error) {
	panic("not written: b-backend")
}

// Put sends PUT with the session cookie and pre-encoded JSON body.
func (b *TestBackend) Put(ctx context.Context, path string, session *Session, body json.RawMessage) (Answer, error) {
	panic("not written: b-backend")
}

// Delete sends DELETE with the session cookie.
func (b *TestBackend) Delete(ctx context.Context, path string, session *Session) (Answer, error) {
	panic("not written: b-backend")
}

// ReadWith sends GET with the session cookie and extra headers.
func (b *TestBackend) ReadWith(ctx context.Context, path string, session *Session, headers http.Header) (Answer, error) {
	panic("not written: b-backend")
}

// Response sends a raw request and transfers the response body to the caller,
// who must close it. A request-body ReadCloser is closed by the HTTP client.
func (b *TestBackend) Response(ctx context.Context, method, path string, session *Session, headers http.Header, body io.Reader) (*http.Response, error) {
	panic("not written: b-backend")
}

// Setup creates and signs in the fixture master account through HTTP.
func (b *TestBackend) Setup(ctx context.Context) (Session, error) { panic("not written: b-backend") }

// Login signs in through HTTP and validates the identity and session cookie.
func (b *TestBackend) Login(ctx context.Context, email, password string) (Session, error) {
	panic("not written: b-backend")
}

// LoginAnswer returns the login response including authentication refusals.
func (b *TestBackend) LoginAnswer(ctx context.Context, email, password string) (Answer, error) {
	panic("not written: b-backend")
}

// Devices returns the session's paired devices as GET /api/devices lists them.
func (b *TestBackend) Devices(ctx context.Context, session *Session) ([]webapi.DeviceDTO, error) {
	panic("not written: b-backend")
}

// Online reports whether the session's device is online in the device list.
func (b *TestBackend) Online(ctx context.Context, session *Session, device webapi.DeviceID) (bool, error) {
	panic("not written: b-backend")
}

// UntilOnline waits for the device's online state through page change events.
func (b *TestBackend) UntilOnline(ctx context.Context, session *Session, device webapi.DeviceID, online bool) error {
	panic("not written: b-backend")
}
