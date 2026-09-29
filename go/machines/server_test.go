//go:build linux

package machines_test

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"io/fs"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/machines"
	"github.com/wspl/demi/go/machinesproto"
)

// scripted records each call and answers it from a script: an operation named in
// failures fails once with that message (keyed by the name of the params type,
// such as HibernateParams), and the call to read the base version waits for
// release when one is set.
type scripted struct {
	mu       sync.Mutex
	calls    []machinesproto.Call
	finished int
	failures map[string]string
	release  chan struct{}
	started  chan struct{}
}

type failure struct {
	message string
	cause   error
}

func (f *failure) Error() string { return f.message }

func (f *failure) Unwrap() error { return f.cause }

func (s *scripted) Handle(_ context.Context, call machinesproto.Call) (any, error) {
	s.mu.Lock()
	s.calls = append(s.calls, call)
	operation := reflect.TypeOf(call).Name()
	message, fails := s.failures[operation]
	delete(s.failures, operation)
	release := s.release
	if _, waits := call.(machinesproto.CurrentBaseVersionParams); waits {
		s.release = nil
	} else {
		release = nil
	}
	s.mu.Unlock()
	if fails {
		return nil, &failure{message: message, cause: errors.New("the cause")}
	}
	if s.started != nil {
		if _, waits := call.(machinesproto.CurrentBaseVersionParams); waits {
			close(s.started)
		}
	}
	if release != nil {
		<-release
	}
	s.mu.Lock()
	s.finished++
	s.mu.Unlock()
	return resultOf(call), nil
}

func resultOf(call machinesproto.Call) any {
	switch call.(type) {
	case machinesproto.CurrentBaseVersionParams:
		return strings.Repeat("b", 64)
	case machinesproto.RuntimeStateParams:
		return "stopped"
	}
	return nil
}

func (s *scripted) called() []machinesproto.Call {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]machinesproto.Call(nil), s.calls...)
}

func (s *scripted) done() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.finished
}

// harness is a server on a socket in a temporary directory.
type harness struct {
	t        *testing.T
	path     string
	service  *scripted
	deaths   chan machinesproto.DeviceID
	stop     context.CancelFunc
	inFlight chan *machines.InFlight
	once     sync.Once
}

func serve(t *testing.T) *harness {
	t.Helper()
	path := filepath.Join(t.TempDir(), "sock/machines.sock")
	socket, err := machines.Bind(path)
	if err != nil {
		t.Fatal(err)
	}
	h := &harness{t: t, path: path, service: &scripted{failures: map[string]string{}}, deaths: make(chan machinesproto.DeviceID, 4), inFlight: make(chan *machines.InFlight, 1)}
	var ctx context.Context
	ctx, h.stop = context.WithCancel(context.Background())
	go func() { h.inFlight <- machines.Serve(ctx, socket, h.service, h.deaths) }()
	t.Cleanup(func() { h.finish() })
	return h
}

// finish stops the server and waits for the requests it left running; a second
// call does nothing more.
func (h *harness) finish() *scripted {
	h.once.Do(func() {
		h.stop()
		close(h.deaths)
		(<-h.inFlight).Wait()
	})
	return h.service
}

type client struct {
	t      *testing.T
	conn   net.Conn
	reader *bufio.Reader
}

func (h *harness) connect() *client {
	h.t.Helper()
	conn, err := net.Dial("unix", h.path)
	if err != nil {
		h.t.Fatal(err)
	}
	h.t.Cleanup(func() { conn.Close() })
	return &client{t: h.t, conn: conn, reader: bufio.NewReader(conn)}
}

func (c *client) send(data string) {
	c.t.Helper()
	if _, err := c.conn.Write([]byte(data)); err != nil {
		c.t.Fatal(err)
	}
}

// receive returns the next message, or nil once the server closed the connection.
// Linux reports a close with unread data as a reset.
func (c *client) receive() machinesproto.Response {
	c.t.Helper()
	// The deadline only turns a lost reply into a failure.
	if err := c.conn.SetReadDeadline(time.Now().Add(10 * time.Second)); err != nil {
		c.t.Fatal(err)
	}
	line, err := c.reader.ReadBytes('\n')
	if err != nil {
		if errors.Is(err, os.ErrDeadlineExceeded) {
			c.t.Fatal("no line in time")
		}
		return nil
	}
	response, err := machinesproto.DecodeResponse(bytes.TrimSuffix(line, []byte("\n")))
	if err != nil {
		c.t.Fatal(err)
	}
	return response
}

func TestEveryCallReachesTheServiceAndItsResultTheClient(t *testing.T) {
	h := serve(t)
	c := h.connect()
	lines := []string{
		`{"id":"1","op":"reconcile","params":{}}`,
		`{"id":"2","op":"current_base_version","params":{}}`,
		`{"id":"3","op":"image_state","params":{"deviceId":"dev-1"}}`,
		`{"id":"4","op":"runtime_state","params":{"deviceId":"dev-1"}}`,
		`{"id":"5","op":"wake","params":{"deviceId":"dev-1","boot":{"backendUrl":"http://backend/","deviceToken":"tok"}}}`,
		`{"id":"6","op":"checkpoint","params":{"deviceId":"dev-1"}}`,
		`{"id":"7","op":"grow_volume","params":{"deviceId":"dev-1","volume":"home","bytes":4096}}`,
		`{"id":"8","op":"reset","params":{"deviceId":"dev-1","operationId":"op-1","baseVersion":"base-2"}}`,
		`{"id":"9","op":"hibernate","params":{"deviceId":"dev-1"}}`,
	}
	var expected []machinesproto.Call
	for _, line := range lines {
		request, err := machinesproto.DecodeRequest([]byte(line))
		if err != nil {
			t.Fatal(err)
		}
		expected = append(expected, request.Call)
		c.send(line + "\n")
		result, err := json.Marshal(resultOf(request.Call))
		if err != nil {
			t.Fatal(err)
		}
		want := machinesproto.OK{ID: request.ID, Result: jsontext.Value(result)}
		if got := c.receive(); !reflect.DeepEqual(got, want) {
			t.Errorf("%T: %+v, want %+v", request.Call, got, want)
		}
	}
	if calls := h.finish().called(); !reflect.DeepEqual(calls, expected) {
		t.Errorf("the service was called with %v", calls)
	}
}

func TestAFailureIsAnErrorReplyAndTheConnectionStaysUsable(t *testing.T) {
	h := serve(t)
	h.service.failures["HibernateParams"] = "no such machine"
	c := h.connect()
	c.send(`{"id":"1","op":"hibernate","params":{"deviceId":"dev-9"}}` + "\n")
	if got, want := c.receive(), (machinesproto.Failure{ID: "1", Message: "no such machine"}); got != want {
		t.Errorf("%+v, want %+v", got, want)
	}
	// An empty line is skipped.
	c.send("\n" + `{"id":"2","op":"reconcile","params":{}}` + "\n")
	if ok, isOK := c.receive().(machinesproto.OK); !isOK || ok.ID != "2" {
		t.Errorf("the request after the failure: %+v", ok)
	}
}

func TestConcurrentRequestsAreAnsweredAsTheyFinish(t *testing.T) {
	h := serve(t)
	release := make(chan struct{})
	h.service.release = release
	c := h.connect()
	c.send(`{"id":"slow","op":"current_base_version","params":{}}` + "\n" + `{"id":"fast","op":"reconcile","params":{}}` + "\n")
	if ok, isOK := c.receive().(machinesproto.OK); !isOK || ok.ID != "fast" {
		t.Errorf("the first reply: %+v", ok)
	}
	close(release)
	if ok, isOK := c.receive().(machinesproto.OK); !isOK || ok.ID != "slow" {
		t.Errorf("the second reply: %+v", ok)
	}
}

func TestADeathReachesEveryConnection(t *testing.T) {
	h := serve(t)
	first, second := h.connect(), h.connect()
	// A round trip on each shows the server has accepted both.
	for _, c := range []*client{first, second} {
		c.send(`{"id":"1","op":"reconcile","params":{}}` + "\n")
		c.receive()
	}
	h.deaths <- "dev-1"
	for _, c := range []*client{first, second} {
		if got, want := c.receive(), (machinesproto.Death{DeviceID: "dev-1"}); got != want {
			t.Errorf("%+v, want %+v", got, want)
		}
	}
}

func TestABadLineDropsItsConnectionBeforeTheLinesBehindIt(t *testing.T) {
	h := serve(t)
	next := `{"id":"2","op":"reconcile","params":{}}` + "\n"
	for _, bad := range []string{
		`{"id":"1","op":"wake","params":{}}` + "\n",
		"not json\n",
		"\xff\n",
		strings.Repeat("x", machinesproto.MaxLineBytes+1) + "\n",
	} {
		c := h.connect()
		// The server may close before it has read everything.
		_, _ = c.conn.Write([]byte(bad + next))
		if got := c.receive(); got != nil {
			t.Errorf("a reply after a bad line: %+v", got)
		}
	}
	if calls := h.finish().called(); len(calls) != 0 {
		t.Errorf("a bad line reached the service: %v", calls)
	}
}

func TestALineOfTheLimitIsServed(t *testing.T) {
	h := serve(t)
	c := h.connect()
	// A request of exactly the limit: padding in a member the contract ignores.
	prefix := `{"id":"1","op":"reconcile","params":{},"x":"`
	suffix := `"}`
	padding := strings.Repeat("y", machinesproto.MaxLineBytes-len(prefix)-len(suffix))
	c.send(prefix + padding + suffix + "\n")
	if ok, isOK := c.receive().(machinesproto.OK); !isOK || ok.ID != "1" {
		t.Errorf("%+v", ok)
	}
}

func TestAReplyForAClosedConnectionIsDiscardedAndItsOperationCompletes(t *testing.T) {
	h := serve(t)
	release := make(chan struct{})
	h.service.release = release
	h.service.started = make(chan struct{})
	c := h.connect()
	c.send(`{"id":"1","op":"current_base_version","params":{}}` + "\n")
	<-h.service.started
	c.conn.Close()
	close(release)
	if finished := h.finish().done(); finished != 1 {
		t.Errorf("%d operations completed", finished)
	}
}

func TestTheSocketReplacesAStaleSocketRefusesOtherFilesAndIsRemovedAtStop(t *testing.T) {
	path := filepath.Join(t.TempDir(), "machines.sock")
	stale, err := net.Listen("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	stale.(*net.UnixListener).SetUnlinkOnClose(false)
	stale.Close()
	socket, err := machines.Bind(path)
	if err != nil {
		t.Fatalf("a stale socket was not replaced: %v", err)
	}
	if info, err := os.Stat(path); err != nil || info.Mode().Perm() != 0o660 {
		t.Errorf("the socket: %v, %v", info, err)
	}
	stopped, stop := context.WithCancel(context.Background())
	stop()
	deaths := make(chan machinesproto.DeviceID)
	close(deaths)
	machines.Serve(stopped, socket, &scripted{}, deaths).Wait()
	if _, err := os.Stat(path); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("the socket file remains: %v", err)
	}
	if err := os.WriteFile(path, []byte("data"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := machines.Bind(path); err == nil {
		t.Error("a file that is no socket was replaced")
	}
	if data, err := os.ReadFile(path); err != nil || string(data) != "data" {
		t.Errorf("the file was touched: %q, %v", data, err)
	}
}
