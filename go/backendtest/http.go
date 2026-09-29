package backendtest

import (
	"bytes"
	"encoding/json/v2"
	"io"
	"net/http"
	"strconv"
	"strings"
	"testing"
)

// A User is an account as the browser sees it.
type User struct {
	ID       string `json:"id"`
	Email    string `json:"email"`
	Nickname string `json:"nickname"`
	Role     string `json:"role"`
}

// A Session is a signed-in browser: its cookie goes on every request.
type Session struct {
	// Cookie is the name and value of the session cookie, as the Cookie header
	// carries them.
	Cookie string
	User   User
}

// A Request is an HTTP request of a scenario. A zero Session sends no cookie.
type Request struct {
	Method  string
	Path    string
	Session *Session
	// Body is sent as JSON; Raw, when set, is sent as it is instead, and Stream
	// as a body of no declared length.
	Body   any
	Raw    []byte
	Stream io.Reader
	// ContentType overrides the body's content type; the empty string keeps
	// JSON for Body and none for Raw.
	ContentType string
	Headers     map[string]string
}

// An Answer is an HTTP answer, read whole.
type Answer struct {
	t      testing.TB
	Status int
	Header http.Header
	Body   []byte
}

// Text is the body as text.
func (a *Answer) Text() string {
	return string(a.Body)
}

// JSON decodes the body into value, or fails the test.
func (a *Answer) JSON(value any) {
	a.t.Helper()
	if err := json.Unmarshal(a.Body, value); err != nil {
		a.t.Fatalf("the answer is not the JSON expected: %v\n%d %s", err, a.Status, a.Body)
	}
}

// Value is the body as loose JSON.
func (a *Answer) Value() any {
	a.t.Helper()
	var value any
	a.JSON(&value)
	return value
}

// At returns the value at a dotted path of the body's JSON, such as
// "conversation.id" or "conversations.0.title", or nil when there is none.
func (a *Answer) At(path string) any {
	a.t.Helper()
	return At(a.Value(), path)
}

// Str is the string at path of the body's JSON, or the empty string.
func (a *Answer) Str(path string) string {
	a.t.Helper()
	text, _ := a.At(path).(string)
	return text
}

// Refusal is the answer's status and its error code.
func (a *Answer) Refusal() (int, string) {
	a.t.Helper()
	return a.Status, a.Str("code")
}

// ErrorCode is the answer's error code, read from a body that is an error.
func (a *Answer) ErrorCode() string {
	var body struct {
		Code string `json:"code"`
	}
	if err := json.Unmarshal(a.Body, &body); err != nil {
		return ""
	}
	return body.Code
}

// ErrorMessage is the answer's error message.
func (a *Answer) ErrorMessage() string {
	a.t.Helper()
	return a.Str("message")
}

// Expect fails the test unless the answer has status, and answers itself.
func (a *Answer) Expect(status int) *Answer {
	a.t.Helper()
	if a.Status != status {
		a.t.Fatalf("the answer is %d, not %d: %s", a.Status, status, a.Body)
	}
	return a
}

// SessionCookies returns the Set-Cookie values that set or clear the session
// cookie.
func (a *Answer) SessionCookies() []string {
	var cookies []string
	for _, value := range a.Header.Values("Set-Cookie") {
		if strings.HasPrefix(value, SessionCookie+"=") {
			cookies = append(cookies, value)
		}
	}
	return cookies
}

// At returns the value at a dotted path of loose JSON, or nil when there is
// none. A path segment names an object's member or a list's index.
func At(value any, path string) any {
	if path == "" {
		return value
	}
	for segment := range strings.SplitSeq(path, ".") {
		switch current := value.(type) {
		case map[string]any:
			value = current[segment]
		case []any:
			index, err := strconv.Atoi(segment)
			if err != nil || index < 0 || index >= len(current) {
				return nil
			}
			value = current[index]
		default:
			return nil
		}
	}
	return value
}

// Do sends the request and reads the answer whole.
func (b *Backend) Do(request Request) *Answer {
	b.t.Helper()
	answer, err := b.TryDo(request)
	if err != nil {
		b.t.Fatalf("%s %s: %v", request.Method, request.Path, err)
	}
	return answer
}

// TryDo sends the request and reads the answer whole, and answers an error
// rather than failing the test, for a goroutine of the scenario.
func (b *Backend) TryDo(request Request) (*Answer, error) {
	response, err := b.Send(request)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		return nil, err
	}
	return &Answer{t: b.t, Status: response.StatusCode, Header: response.Header, Body: body}, nil
}

// Send sends the request and answers the response before its body is read.
func (b *Backend) Send(request Request) (*http.Response, error) {
	var body io.Reader
	contentType := request.ContentType
	switch {
	case request.Stream != nil:
		// A reader os/http cannot measure goes out chunked.
		body = io.NopCloser(request.Stream)
	case request.Raw != nil:
		body = bytes.NewReader(request.Raw)
	case request.Body != nil:
		encoded, err := json.Marshal(request.Body, json.Deterministic(true))
		if err != nil {
			return nil, err
		}
		body = bytes.NewReader(encoded)
		if contentType == "" {
			contentType = "application/json"
		}
	}
	method := request.Method
	if method == "" {
		method = http.MethodGet
	}
	built, err := http.NewRequest(method, b.URL+request.Path, body)
	if err != nil {
		return nil, err
	}
	if request.Session != nil {
		built.Header.Set("Cookie", request.Session.Cookie)
	}
	if contentType != "" {
		built.Header.Set("Content-Type", contentType)
	}
	for name, value := range request.Headers {
		built.Header.Set(name, value)
	}
	return b.client.Do(built)
}

func (b *Backend) read(response *http.Response) *Answer {
	b.t.Helper()
	defer response.Body.Close()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		b.t.Fatalf("the answer's body: %v", err)
	}
	return &Answer{t: b.t, Status: response.StatusCode, Header: response.Header, Body: body}
}

// Read reads a response from Send whole.
func (b *Backend) Read(response *http.Response) *Answer {
	b.t.Helper()
	return b.read(response)
}

// Get sends a GET with the session's cookie, if any.
func (b *Backend) Get(path string, session *Session) *Answer {
	b.t.Helper()
	return b.Do(Request{Method: http.MethodGet, Path: path, Session: session})
}

// Post sends a POST of body as JSON.
func (b *Backend) Post(path string, session *Session, body any) *Answer {
	b.t.Helper()
	return b.Do(Request{Method: http.MethodPost, Path: path, Session: session, Body: body})
}

// Patch sends a PATCH of body as JSON.
func (b *Backend) Patch(path string, session *Session, body any) *Answer {
	b.t.Helper()
	return b.Do(Request{Method: http.MethodPatch, Path: path, Session: session, Body: body})
}

// Put sends a PUT of body as JSON.
func (b *Backend) Put(path string, session *Session, body any) *Answer {
	b.t.Helper()
	return b.Do(Request{Method: http.MethodPut, Path: path, Session: session, Body: body})
}

// Delete sends a DELETE.
func (b *Backend) Delete(path string, session *Session) *Answer {
	b.t.Helper()
	return b.Do(Request{Method: http.MethodDelete, Path: path, Session: session})
}

// Map is loose JSON object, for the bodies a scenario sends.
type Map = map[string]any

// Setup sets the instance up with the master account and signs it in.
func (b *Backend) Setup() *Session {
	b.t.Helper()
	answer := b.Post("/api/setup", nil, Map{"email": MasterEmail, "password": MasterPassword})
	answer.Expect(http.StatusCreated)
	return b.sessionFrom(answer)
}

// Login signs an account in, and fails the test unless it succeeds.
func (b *Backend) Login(email, password string) *Session {
	b.t.Helper()
	answer := b.LoginAnswer(email, password)
	answer.Expect(http.StatusOK)
	return b.sessionFrom(answer)
}

// LoginAnswer is the answer of a sign-in.
func (b *Backend) LoginAnswer(email, password string) *Answer {
	b.t.Helper()
	return b.Post("/api/auth/login", nil, Map{"email": email, "password": password})
}

// sessionFrom is the session a setup or login answer signed in.
func (b *Backend) sessionFrom(answer *Answer) *Session {
	b.t.Helper()
	cookies := answer.SessionCookies()
	if len(cookies) != 1 {
		b.t.Fatalf("expected one session cookie, got %v", cookies)
	}
	pair, _, _ := strings.Cut(cookies[0], ";")
	var identity struct {
		User User `json:"user"`
	}
	answer.JSON(&identity)
	return &Session{Cookie: pair, User: identity.User}
}

// CreateUser creates an account as actor, who must administer its role, and
// signs it in.
func (b *Backend) CreateUser(actor *Session, email, password, role string) *Session {
	b.t.Helper()
	created := b.Post("/api/users", actor, Map{"email": email, "password": password, "role": role})
	created.Expect(http.StatusCreated)
	return b.Login(email, password)
}

// SessionFromAnswer is the session a setup or login answer signed in.
func (b *Backend) SessionFromAnswer(answer *Answer) *Session {
	b.t.Helper()
	return b.sessionFrom(answer)
}
