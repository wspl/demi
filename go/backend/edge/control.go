//go:build testing

package edge

// What a test build adds for a black-box suite that runs demi-backend as a
// process (docs/internal/go-migration/design/g6-api-suite.md): the tuning
// file DEMI_TEST_TUNING and the control socket DEMI_TEST_CONTROL, whose
// messages controlproto defines. A release build has none of it.
//
// The tuning file sets what no environment variable sets; a member left out
// keeps the product's default, and an unknown one refuses the file. The
// control socket carries newline-delimited JSON, one request and one reply
// per line, replies in the order the requests finish. It accepts
// connections once the backend serves, so its existence tells a suite that
// the backend is ready.

import (
	"bufio"
	"context"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"net"
	"net/url"
	"os"
	"runtime"
	"sync"
	"sync/atomic"
	"time"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/backend/auth"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/backendtest/controlproto"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/fsfail"
	"github.com/wspl/demi/go/webapi"
)

// clockStart is where the manual clock starts: the start every suite
// shares.
const clockStart = "2026-09-24T08:00:00Z"

// emptyResult is the result of an operation that answers nothing but that
// it succeeded.
var emptyResult = jsontext.Value("{}")

// TestControl is the hooks a test build gives a suite, set up before the
// backend starts and served on the control socket once it does.
type TestControl struct {
	socket string
	clock  *manualClock
	mail   *mailbox
	counts *objectCounts

	listener net.Listener
	// accepted closes once the accepting goroutine returned.
	accepted chan struct{}
	// ending ends every connection, at End.
	ending context.Context
	end    context.CancelFunc
	// connections are the open connections.
	connections sync.WaitGroup
	// mu guards server, which Stop takes away: a request that needs the
	// backend then answers that it is closing.
	mu     sync.RWMutex
	server *Server
}

func tuningError(path, reason string) error {
	return fmt.Errorf("%s (%s) cannot be used: %s", controlproto.TuningVariable, path, reason)
}

// ControlFromEnvironment reads the tuning file DEMI_TEST_TUNING names into
// config and, when DEMI_TEST_CONTROL names a socket, puts the backend on a
// manual clock, over an object store that counts, and behind a mailbox when
// the tuning asks for mail, and answers the control that serves them; nil
// when no socket is named.
func ControlFromEnvironment(config *Config) (*TestControl, error) {
	var tuning controlproto.Tuning
	if path, ok := os.LookupEnv(controlproto.TuningVariable); ok {
		data, err := os.ReadFile(path)
		if err != nil {
			return nil, tuningError(path, fsfail.Cause(err).Error())
		}
		tuning, err = controlproto.DecodeTuning(data)
		if err != nil {
			return nil, tuningError(path, err.Error())
		}
		if err := applyTuning(tuning, config); err != nil {
			return nil, tuningError(path, err.Error())
		}
	}
	socket, ok := os.LookupEnv(controlproto.ControlVariable)
	if !ok {
		return nil, nil
	}
	start, err := core.ParseTimestamp(clockStart)
	if err != nil {
		// The start is a timestamp.
		panic(err)
	}
	if tuning.ClockStartMs != nil {
		start, err = core.TimestampFromMillisecond(*tuning.ClockStartMs)
		if err != nil {
			return nil, tuningError(controlproto.TuningVariable, err.Error())
		}
	}
	ending, end := context.WithCancel(context.Background())
	control := &TestControl{
		socket:   socket,
		clock:    &manualClock{now: start},
		mail:     &mailbox{},
		counts:   &objectCounts{},
		accepted: make(chan struct{}),
		ending:   ending,
		end:      end,
	}
	config.Clock = control.clock
	if tuning.Mail != nil && *tuning.Mail {
		config.AccountMail = control.mail
	}
	config.ObserveObjects = control.counts.observe
	return control, nil
}

// applyTuning sets what the tuning names in config, but the clock and the
// mail sender, which the control sets up. The members of the parts the Go
// backend does not have yet (runners, lifecycle, the Cloud's times,
// conversations, pages, exposes and user streams) are read and checked, and
// apply once those parts land.
func applyTuning(tuning controlproto.Tuning, config *Config) error {
	if logins := tuning.Logins; logins != nil {
		if logins.LifetimeMs != nil {
			config.Logins.Lifetime = millis(*logins.LifetimeMs)
		}
		if logins.RetentionMs != nil {
			config.Logins.Retention = millis(*logins.RetentionMs)
		}
	}
	if cloud := tuning.Cloud; cloud != nil && cloud.Capacity != nil {
		if *cloud.Capacity > math.MaxInt64 {
			return errors.New("cloud.capacity is over the most a count holds")
		}
		config.CloudCapacity = int64(*cloud.Capacity)
	}
	if families := tuning.Families; families != nil && families.Codex != nil {
		endpoints, err := codexEndpoints(*families.Codex)
		if err != nil {
			return err
		}
		config.Families = backend.BuiltinFamilies().With("codex", backend.CodexFamily{Endpoints: endpoints})
	}
	if tuning.ModelsDevURL != nil {
		config.ModelsDevURL = *tuning.ModelsDevURL
	}
	return nil
}

// codexEndpoints are the scripted servers the codex family reaches in place
// of its vendor's.
func codexEndpoints(tuning controlproto.CodexTuning) (*backend.CodexEndpoints, error) {
	var endpoints backend.CodexEndpoints
	for _, endpoint := range []struct {
		name, text string
		target     *url.URL
	}{
		{"families.codex.backendUrl", tuning.BackendURL, &endpoints.Backend},
		{"families.codex.authUrl", tuning.AuthURL, &endpoints.Auth},
	} {
		address, err := core.ParseURL(endpoint.text)
		if err != nil {
			return nil, fmt.Errorf("%s %w", endpoint.name, err)
		}
		parsed, err := core.NetURL(address)
		if err != nil {
			return nil, fmt.Errorf("%s %w", endpoint.name, err)
		}
		*endpoint.target = *parsed
	}
	return &endpoints, nil
}

func millis(value uint64) time.Duration {
	return time.Duration(value) * time.Millisecond
}

// manualClock is wall-clock time a test sets: it stands still until it is
// moved.
type manualClock struct {
	mu  sync.Mutex
	now core.Timestamp
}

func (c *manualClock) Now() core.Timestamp {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}

func (c *manualClock) set(at core.Timestamp) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.now = at
}

// advance moves the clock by byMs milliseconds, which may be negative, and
// answers its new time.
func (c *manualClock) advance(byMs int64) (core.Timestamp, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	now := c.now.Millisecond()
	if byMs > 0 && now > math.MaxInt64-byMs || byMs < 0 && now < math.MinInt64-byMs {
		return core.Timestamp{}, errors.New("outside the supported range of times")
	}
	moved, err := core.TimestampFromMillisecond(now + byMs)
	if err != nil {
		return core.Timestamp{}, err
	}
	c.now = moved
	return moved, nil
}

// mailbox captures verification mail, or refuses it while failing is set.
type mailbox struct {
	mu      sync.Mutex
	sent    []auth.VerificationMail
	failing atomic.Bool
}

func (m *mailbox) SendVerification(_ context.Context, mail auth.VerificationMail) error {
	if m.failing.Load() {
		return errors.New("the mail transport failed")
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	m.sent = append(m.sent, mail)
	return nil
}

// list is the mail sent so far, in order.
func (m *mailbox) list() controlproto.MailList {
	m.mu.Lock()
	defer m.mu.Unlock()
	list := controlproto.MailList{Mail: []controlproto.Mail{}}
	for _, mail := range m.sent {
		list.Mail = append(list.Mail, controlproto.Mail{Email: mail.Email.String(), Code: mail.Code, ExpiresAtMs: mail.ExpiresAt.Millisecond()})
	}
	return list
}

// objectCounts counts what reaches the object store, for the scenarios that
// prove what the backend reads and writes (storage.md § The object store, §
// Retention): each put with its bytes, each read, each HEAD, the most reads
// in flight at once, each listing and each deletion.
type objectCounts struct {
	puts, bytesPut, gets, heads, reading, mostReading, lists, deletes atomic.Uint64
}

// observe is objects, with what reaches it counted here.
func (c *objectCounts) observe(objects storage.ObjectStore) storage.ObjectStore {
	return &countedObjects{inner: objects, counts: c}
}

func (c *objectCounts) tally() controlproto.ObjectTally {
	return controlproto.ObjectTally{
		Puts:           c.puts.Load(),
		BytesPut:       c.bytesPut.Load(),
		Gets:           c.gets.Load(),
		Heads:          c.heads.Load(),
		MostGetsAtOnce: c.mostReading.Load(),
		Lists:          c.lists.Load(),
		Deletes:        c.deletes.Load(),
	}
}

// read counts a read that is in flight until the function it answers is
// called.
func (c *objectCounts) read() (done func()) {
	c.gets.Add(1)
	now := c.reading.Add(1)
	for {
		most := c.mostReading.Load()
		if now <= most || c.mostReading.CompareAndSwap(most, now) {
			break
		}
	}
	return func() { c.reading.Add(^uint64(0)) }
}

type countedObjects struct {
	inner  storage.ObjectStore
	counts *objectCounts
}

func (o *countedObjects) Head(ctx context.Context, key string) (storage.ObjectMeta, error) {
	o.counts.heads.Add(1)
	return o.inner.Head(ctx, key)
}

func (o *countedObjects) Get(ctx context.Context, key string) ([]byte, error) {
	done := o.counts.read()
	defer done()
	// A read takes a moment, as it does over a network. A local file can
	// answer before the caller starts its next read, and a loaded machine
	// makes that likely, so without this yield reads that start together
	// would count as one at a time.
	runtime.Gosched()
	return o.inner.Get(ctx, key)
}

func (o *countedObjects) Create(ctx context.Context, key string, data []byte, metadata map[string]string) error {
	o.counts.puts.Add(1)
	o.counts.bytesPut.Add(uint64(len(data)))
	return o.inner.Create(ctx, key, data, metadata)
}

func (o *countedObjects) List(ctx context.Context, prefix string) ([]storage.ObjectMeta, error) {
	o.counts.lists.Add(1)
	return o.inner.List(ctx, prefix)
}

func (o *countedObjects) Delete(ctx context.Context, key string) error {
	o.counts.deletes.Add(1)
	return o.inner.Delete(ctx, key)
}

// Serve binds the control socket and serves it over server until Stop.
// It binds once the backend serves: a connection to it tells a suite that
// the backend is ready.
func (c *TestControl) Serve(server *Server) error {
	// A socket a killed backend left behind.
	_ = os.Remove(c.socket)
	listener, err := net.Listen("unix", c.socket)
	if err != nil {
		return fmt.Errorf("%s (%s) cannot be bound: %w", controlproto.ControlVariable, c.socket, err)
	}
	c.listener = listener
	c.server = server
	go c.accept()
	return nil
}

// Stop closes the socket to new connections and takes the backend away for
// its shutdown. What the connections hold stays held: a request that needs
// the backend now answers that it is closing.
func (c *TestControl) Stop() {
	// Closing the listener ends the accepting goroutine; its error is the
	// one it answers.
	_ = c.listener.Close()
	<-c.accepted
	// A socket that is gone already has nothing to remove.
	_ = os.Remove(c.socket)
	c.mu.Lock()
	c.server = nil
	c.mu.Unlock()
}

// End closes every connection, which releases what they hold, and waits for
// them.
func (c *TestControl) End() {
	c.end()
	c.connections.Wait()
}

func (c *TestControl) accept() {
	defer close(c.accepted)
	for {
		conn, err := c.listener.Accept()
		if err != nil {
			if !errors.Is(err, net.ErrClosed) {
				slog.Error("the test control cannot accept", "error", err)
			}
			return
		}
		c.connections.Add(1)
		go func() {
			defer c.connections.Done()
			c.connection(conn)
		}()
	}
}

// connection serves one control connection: requests run concurrently and
// are answered as they finish. A line that is not a request drops the
// connection, and a request still running ends with it.
func (c *TestControl) connection(conn net.Conn) {
	ctx, cancel := context.WithCancel(c.ending)
	defer cancel()
	// Ending the connection ends the read that waits for its next line.
	unblock := context.AfterFunc(ctx, func() { _ = conn.Close() })
	defer unblock()
	replies := make(chan []byte)
	written := make(chan struct{})
	go func() {
		defer close(written)
		for line := range replies {
			if _, err := conn.Write(line); err != nil {
				// A connection that closed discards its replies.
				cancel()
			}
		}
	}()
	var requests sync.WaitGroup
	lines := bufio.NewScanner(conn)
	lines.Buffer(make([]byte, 0, 64*1024), controlproto.MaxLineBytes+1)
	for lines.Scan() {
		line := lines.Bytes()
		if len(line) == 0 {
			continue
		}
		if len(line) > controlproto.MaxLineBytes {
			break
		}
		request, err := controlproto.DecodeRequest(line)
		if err != nil {
			break
		}
		requests.Go(func() {
			reply := c.reply(ctx, request)
			select {
			case replies <- reply:
			case <-ctx.Done():
			}
		})
	}
	cancel()
	requests.Wait()
	close(replies)
	<-written
	// The connection may be closed already by the end of the control.
	_ = conn.Close()
}

// reply runs the request and answers its reply line.
func (c *TestControl) reply(ctx context.Context, request controlproto.Request) []byte {
	var response controlproto.Response
	result, err := c.run(ctx, request.Call)
	if err == nil {
		response = controlproto.OK{ID: request.ID, Result: result}
	} else {
		response = controlproto.Failure{ID: request.ID, Message: err.Error()}
	}
	line, err := controlproto.EncodeResponse(response)
	if err != nil {
		// A reply of the package's own types encodes.
		panic(err)
	}
	return line
}

// run runs one operation of the control and answers its result.
func (c *TestControl) run(ctx context.Context, call controlproto.Call) (jsontext.Value, error) {
	switch call := call.(type) {
	case controlproto.ClockAdvanceParams:
		moved, err := c.clock.advance(call.ByMs)
		if err != nil {
			return nil, err
		}
		return controlproto.EncodeResult(controlproto.ClockResult{AtMs: moved.Millisecond()})
	case controlproto.ClockSetParams:
		at, err := core.TimestampFromMillisecond(call.AtMs)
		if err != nil {
			return nil, err
		}
		c.clock.set(at)
		return controlproto.EncodeResult(controlproto.ClockResult{AtMs: at.Millisecond()})
	case controlproto.UsersAddParams:
		id, err := c.addUser(ctx, call)
		if err != nil {
			return nil, err
		}
		return controlproto.EncodeResult(controlproto.IDResult{ID: id.String()})
	case controlproto.ObjectsCountParams:
		return controlproto.EncodeResult(c.counts.tally())
	case controlproto.MailListParams:
		return controlproto.EncodeResult(c.mail.list())
	case controlproto.MailFailParams:
		c.mail.failing.Store(call.Failing)
		return emptyResult, nil
	}
	// The holds, the file gate's leases and the retention pass come with
	// the parts of the backend they hold (G7g).
	return nil, fmt.Errorf("the Go backend does not serve %T yet", call)
}

// addUser adds an account without a route of the API, as the API would
// create it.
func (c *TestControl) addUser(ctx context.Context, add controlproto.UsersAddParams) (webapi.UserID, error) {
	email, err := webapi.ParseEmailAddress(add.Email)
	if err != nil {
		return webapi.UserID{}, err
	}
	c.mu.RLock()
	defer c.mu.RUnlock()
	if c.server == nil {
		return webapi.UserID{}, errors.New("the backend is closing")
	}
	user, err := c.server.createAccount(ctx, email, webapi.Password(add.Password), webapi.Role(add.Role))
	if err != nil {
		return webapi.UserID{}, errors.New(answerOf(err).body.Message)
	}
	return user.ID, nil
}
