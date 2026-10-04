package backendtest

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/netip"
	"strings"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapiproto"
)

// TestBackend is a running fixture and its HTTP client. Harness.Start owns
// its cleanup; Close also permits restart over the same durable data.
type TestBackend struct {
	// URL is the HTTP origin of the running backend.
	URL string
	// Backend owns the running backend services and storage.
	Backend *backend.Backend
	// HTTP is the fixture client whose idle connections are closed at cleanup.
	HTTP *http.Client
}

// Session is a signed-in user's session, as their browser holds it: its cookie
// on every request.
type Session struct {
	// Cookie is the session cookie sent with browser requests.
	Cookie string
	// User is the validated signed-in user.
	User webapiproto.UserDTO
}

// Answer is an HTTP answer, read whole.
type Answer struct {
	// Status is the HTTP response status code.
	Status int
	// Headers contains the response headers.
	Headers http.Header
	// Body contains the complete response body.
	Body []byte
}

// DecodeAnswer decodes an answer using its contract's generated decoder.
// Supplying the decoder keeps validation at entry; no reflection decoder is used.
func DecodeAnswer[T any](answer Answer, decode func([]byte) (T, error)) (T, error) {
	return decode(answer.Body)
}

// ErrorBody validates and decodes the backend's error contract.
func (a Answer) ErrorBody() (webapiproto.ErrorBody, error) {
	return webapiproto.DecodeErrorBody(a.Body)
}

// Refusal returns the answer's HTTP status and validated error code.
func (a Answer) Refusal() (int, webapiproto.ErrorCode, error) {
	body, err := a.ErrorBody()
	return a.Status, body.Code, err
}

// SessionCookies returns Set-Cookie values that set or clear the session cookie.
func (a Answer) SessionCookies() []string {
	var cookies []string
	for _, value := range a.Headers.Values("Set-Cookie") {
		if strings.HasPrefix(value, SessionCookie+"=") {
			cookies = append(cookies, value)
		}
	}
	return cookies
}

// ReadAnswer reads the whole response and closes its body on every exit.
func ReadAnswer(ctx context.Context, response *http.Response) (answer Answer, err error) {
	defer func() { err = errors.Join(err, response.Body.Close()) }()
	if err := ctx.Err(); err != nil {
		return Answer{}, err
	}
	data, err := io.ReadAll(response.Body)
	return Answer{Status: response.StatusCode, Headers: response.Header.Clone(), Body: data}, err
}

// SessionFrom validates a setup or login identity and its one session cookie.
func SessionFrom(answer Answer) (Session, error) {
	cookies := answer.SessionCookies()
	if len(cookies) != 1 {
		return Session{}, fmt.Errorf("expected one session cookie, got %v", cookies)
	}
	identity, err := webapiproto.DecodeIdentity(answer.Body)
	if err != nil {
		return Session{}, err
	}
	cookie, _, _ := strings.Cut(cookies[0], ";")
	return Session{Cookie: cookie, User: identity.User}, nil
}

// Close joins backend shutdown and reports all failed steps.
func (b *TestBackend) Close(ctx context.Context) error {
	defer b.HTTP.CloseIdleConnections()
	return b.Backend.Close(ctx)
}

// Address returns the listener's bound address.
func (b *TestBackend) Address() netip.AddrPort { return b.Backend.LocalAddr() }

// WSURL returns the ws:// URL of path on this backend.
func (b *TestBackend) WSURL(path string) string {
	return "ws" + strings.TrimPrefix(b.URL, "http") + path
}

// Send sends a request with an optional cookie and pre-encoded JSON body.
// A nil body sends no JSON. Callers use generated encoders or contract.EncodeJSON.
func (b *TestBackend) Send(
	ctx context.Context,
	method, path string,
	cookie *string,
	body json.RawMessage,
) (Answer, error) {
	var reader io.Reader
	if body != nil {
		reader = bytes.NewReader(body)
	}
	request, err := http.NewRequestWithContext(ctx, method, b.URL+path, reader)
	if err != nil {
		return Answer{}, err
	}
	if cookie != nil {
		request.Header.Set("Cookie", *cookie)
	}
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	response, err := b.HTTP.Do(request)
	if err != nil {
		return Answer{}, err
	}
	return ReadAnswer(ctx, response)
}

// Read sends GET with the optional browser session cookie.
func (b *TestBackend) Read(ctx context.Context, path string, session *Session) (Answer, error) {
	return b.sendSession(ctx, http.MethodGet, path, session, nil)
}

// Post sends POST with the optional session cookie and pre-encoded JSON body.
func (b *TestBackend) Post(ctx context.Context, path string, session *Session, body json.RawMessage) (Answer, error) {
	return b.sendSession(ctx, http.MethodPost, path, session, body)
}

// Patch sends PATCH with the session cookie and pre-encoded JSON body.
func (b *TestBackend) Patch(ctx context.Context, path string, session *Session, body json.RawMessage) (Answer, error) {
	return b.sendSession(ctx, http.MethodPatch, path, session, body)
}

// Put sends PUT with the session cookie and pre-encoded JSON body.
func (b *TestBackend) Put(ctx context.Context, path string, session *Session, body json.RawMessage) (Answer, error) {
	return b.sendSession(ctx, http.MethodPut, path, session, body)
}

// Delete sends DELETE with the session cookie.
func (b *TestBackend) Delete(ctx context.Context, path string, session *Session) (Answer, error) {
	return b.sendSession(ctx, http.MethodDelete, path, session, nil)
}

// ReadWith sends GET with the session cookie and extra headers.
func (b *TestBackend) ReadWith(
	ctx context.Context,
	path string,
	session *Session,
	headers http.Header,
) (Answer, error) {
	response, err := b.Response(ctx, http.MethodGet, path, session, headers, nil)
	if err != nil {
		return Answer{}, err
	}
	return ReadAnswer(ctx, response)
}

// Response sends a raw request and transfers the response body to the caller,
// who must close it. A request-body ReadCloser is closed by the HTTP client.
func (b *TestBackend) Response(
	ctx context.Context,
	method, path string,
	session *Session,
	headers http.Header,
	body io.Reader,
) (*http.Response, error) {
	request, err := http.NewRequestWithContext(ctx, method, b.URL+path, body)
	if err != nil {
		return nil, err
	}
	request.Header = headers.Clone()
	if request.Header == nil {
		request.Header = make(http.Header)
	}
	if session != nil {
		request.Header.Set("Cookie", session.Cookie)
	}
	return b.HTTP.Do(request)
}

// Setup creates and signs in the fixture master account through HTTP.
func (b *TestBackend) Setup(ctx context.Context) (Session, error) {
	email, err := webapiproto.ParseEmailAddress(MasterEmail)
	if err != nil {
		return Session{}, err
	}
	body, err := contract.EncodeJSON(
		webapiproto.SetupRequest{Email: email, Password: webapiproto.Password(MasterPassword)},
	)
	if err != nil {
		return Session{}, err
	}
	answer, err := b.Post(ctx, "/api/setup", nil, body)
	if err != nil {
		return Session{}, err
	}
	if answer.Status != http.StatusCreated {
		return Session{}, fmt.Errorf("setup: HTTP %d: %s", answer.Status, answer.Body)
	}
	return SessionFrom(answer)
}

// Login signs in through HTTP and validates the identity and session cookie.
func (b *TestBackend) Login(ctx context.Context, email, password string) (Session, error) {
	answer, err := b.LoginAnswer(ctx, email, password)
	if err != nil {
		return Session{}, err
	}
	if answer.Status != http.StatusOK {
		return Session{}, fmt.Errorf("login: HTTP %d: %s", answer.Status, answer.Body)
	}
	return SessionFrom(answer)
}

// LoginAnswer returns the login response including authentication refusals.
func (b *TestBackend) LoginAnswer(ctx context.Context, email, password string) (Answer, error) {
	// Deliberately invalid credentials must reach the server as they are in
	// refusal tests; do not validate or normalize them.
	body, err := contract.EncodeObject(
		[]contract.Field{{Name: "email", Value: email}, {Name: "password", Value: password}},
	)
	if err != nil {
		return Answer{}, err
	}
	return b.Post(ctx, "/api/auth/login", nil, body)
}

// Devices returns the session's paired devices as GET /api/devices lists them.
func (b *TestBackend) Devices(ctx context.Context, session *Session) ([]webapiproto.DeviceDTO, error) {
	answer, err := b.Read(ctx, "/api/devices", session)
	if err != nil {
		return nil, err
	}
	if answer.Status != http.StatusOK {
		return nil, fmt.Errorf("devices: HTTP %d: %s", answer.Status, answer.Body)
	}
	devices, err := webapiproto.DecodeDevices(answer.Body)
	return devices.Devices, err
}

// Online reports whether the session's device is online in the device list.
func (b *TestBackend) Online(ctx context.Context, session *Session, device webapiproto.DeviceID) (bool, error) {
	devices, err := b.Devices(ctx, session)
	if err != nil {
		return false, err
	}
	for _, value := range devices {
		if value.ID == device && value.Online {
			return true, nil
		}
	}
	return false, nil
}

// UntilOnline waits for the device's online state through page change events.
func (b *TestBackend) UntilOnline(
	ctx context.Context,
	session *Session,
	device webapiproto.DeviceID,
	online bool,
) error {
	channel, err := b.sync(ctx, session)
	if err != nil {
		return err
	}
	defer func() {
		// Closing after a read failure is teardown; the read owns its diagnostic.
		_ = channel.Close(context.WithoutCancel(ctx))
	}()
	for {
		event, err := channel.Next(ctx)
		if err != nil {
			return err
		}
		var devices []webapiproto.DeviceDTO
		switch event := event.(type) {
		case *webapiproto.SyncEventSnapshot:
			devices = event.State.Devices
		case *webapiproto.SyncEventDevices:
			devices = event.Devices
		case *webapiproto.SyncEventCloud,
			*webapiproto.SyncEventConversation,
			*webapiproto.SyncEventConversationOrder,
			*webapiproto.SyncEventHeartbeat,
			*webapiproto.SyncEventPlugin,
			*webapiproto.SyncEventPlugins,
			*webapiproto.SyncEventPreferences,
			*webapiproto.SyncEventProviders,
			*webapiproto.SyncEventUser,
			*webapiproto.SyncEventWorkspaces:
			continue
		}
		found := false
		for _, value := range devices {
			if value.ID == device {
				found = value.Online
			}
		}
		if found == online {
			return nil
		}
	}
}

// sendSession supplies the optional browser cookie to the fixture request.
func (b *TestBackend) sendSession(
	ctx context.Context,
	method, path string,
	session *Session,
	body json.RawMessage,
) (Answer, error) {
	var cookie *string
	if session != nil {
		cookie = &session.Cookie
	}
	return b.Send(ctx, method, path, cookie, body)
}
