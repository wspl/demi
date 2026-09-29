package backendtest

import (
	"bufio"
	"context"
	"encoding/json/jsontext"
	"fmt"
	"net"
	"strconv"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest/controlproto"
)

// Patience is how long a scenario waits for something that should come true
// before it fails as hung.
const Patience = 20 * time.Second

// A Control is a client of a test build's control socket. A hold or a lease it
// takes belongs to its connection and ends when it is released or the backend
// stops, so a failed scenario leaves none behind.
type Control struct {
	t    testing.TB
	conn net.Conn

	writing sync.Mutex

	mu      sync.Mutex
	next    int
	pending map[string]chan controlproto.Response
	// ended is set once the connection ended, and failure is why.
	ended   bool
	failure error
}

// dialControl connects to the control socket at path.
func dialControl(t testing.TB, path string) (*Control, error) {
	conn, err := net.DialTimeout("unix", path, time.Second)
	if err != nil {
		return nil, err
	}
	c := &Control{t: t, conn: conn, pending: map[string]chan controlproto.Response{}}
	go c.read()
	return c, nil
}

// read delivers replies to their requests, and ends every request that is
// waiting when the connection ends.
func (c *Control) read() {
	reader := bufio.NewReaderSize(c.conn, 1<<16)
	var failure error
	for {
		line, err := reader.ReadBytes('\n')
		if err != nil {
			failure = err
			break
		}
		response, err := controlproto.DecodeResponse(line[:len(line)-1])
		if err != nil {
			failure = err
			break
		}
		id := responseID(response)
		c.mu.Lock()
		reply := c.pending[id]
		delete(c.pending, id)
		c.mu.Unlock()
		if reply != nil {
			reply <- response
		}
	}
	c.mu.Lock()
	c.ended = true
	c.failure = failure
	for id, reply := range c.pending {
		close(reply)
		delete(c.pending, id)
	}
	c.mu.Unlock()
}

func responseID(response controlproto.Response) string {
	switch response := response.(type) {
	case controlproto.OK:
		return response.ID
	case controlproto.Failure:
		return response.ID
	}
	return ""
}

// Close ends the connection, and with it every hold and lease it took.
func (c *Control) Close() {
	// The socket ends with the backend; a close that fails leaves nothing.
	_ = c.conn.Close()
}

// forget drops the request that will not be answered.
func (c *Control) forget(id string) {
	c.mu.Lock()
	delete(c.pending, id)
	c.mu.Unlock()
}

// call sends the operation and waits for its result.
func (c *Control) call(ctx context.Context, operation controlproto.Call) (jsontext.Value, error) {
	c.mu.Lock()
	if c.ended {
		defer c.mu.Unlock()
		return nil, fmt.Errorf("the control socket ended: %w", c.failure)
	}
	c.next++
	id := strconv.Itoa(c.next)
	reply := make(chan controlproto.Response, 1)
	c.pending[id] = reply
	c.mu.Unlock()
	line, err := controlproto.EncodeRequest(controlproto.Request{ID: id, Call: operation})
	if err != nil {
		c.forget(id)
		return nil, err
	}
	c.writing.Lock()
	_, err = c.conn.Write(line)
	c.writing.Unlock()
	if err != nil {
		c.forget(id)
		return nil, err
	}
	select {
	case response, ok := <-reply:
		if !ok {
			c.mu.Lock()
			defer c.mu.Unlock()
			return nil, fmt.Errorf("the control socket ended: %w", c.failure)
		}
		switch response := response.(type) {
		case controlproto.OK:
			return response.Result, nil
		case controlproto.Failure:
			return nil, fmt.Errorf("the control refused: %s", response.Message)
		}
		return nil, fmt.Errorf("the control answered %T", response)
	case <-ctx.Done():
		c.forget(id)
		return nil, ctx.Err()
	}
}

// do sends the operation and fails the test when it does not answer in time.
func (c *Control) do(operation controlproto.Call) jsontext.Value {
	c.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	result, err := c.call(ctx, operation)
	if err != nil {
		c.t.Fatalf("%s: %v", controlOperation(operation), err)
	}
	return result
}

func controlOperation(operation controlproto.Call) string {
	return fmt.Sprintf("%T", operation)
}

// AdvanceClock moves the backend's manual clock, which stands still until it is
// moved, and answers its new time.
func (c *Control) AdvanceClock(by time.Duration) time.Time {
	c.t.Helper()
	result := c.do(controlproto.ClockAdvanceParams{ByMs: by.Milliseconds()})
	return c.clock(result)
}

// SetClock sets the backend's manual clock.
func (c *Control) SetClock(at time.Time) {
	c.t.Helper()
	c.clock(c.do(controlproto.ClockSetParams{AtMs: at.UnixMilli()}))
}

func (c *Control) clock(result jsontext.Value) time.Time {
	c.t.Helper()
	moved, err := controlproto.DecodeResult[controlproto.ClockResult](result)
	if err != nil {
		c.t.Fatal(err)
	}
	return time.UnixMilli(moved.AtMs).UTC()
}

// AddUser adds an account without a route of the API and answers its id.
func (c *Control) AddUser(email, password, role string) string {
	c.t.Helper()
	return c.id(c.do(controlproto.UsersAddParams{Email: email, Password: password, Role: role}))
}

func (c *Control) id(result jsontext.Value) string {
	c.t.Helper()
	named, err := controlproto.DecodeResult[controlproto.IDResult](result)
	if err != nil {
		c.t.Fatal(err)
	}
	return named.ID
}

// A Lease is a lease of a conversation's file gate that a control connection
// holds.
type Lease struct {
	control *Control
	id      string
}

// EnterGate enters the file gate of the user's conversation with a lease for
// demand ("demand") or system work ("maintenance"), and answers once it is held.
func (c *Control) EnterGate(user, conversation, purpose string) *Lease {
	c.t.Helper()
	id := c.id(c.do(controlproto.GateEnterParams{User: user, Conversation: conversation, Purpose: purpose}))
	return &Lease{control: c, id: id}
}

// Release ends the lease.
func (l *Lease) Release() {
	l.control.t.Helper()
	l.control.do(controlproto.ReleaseParams{ID: l.id})
}

// WaitGateEntrants waits until count entrants wait at the file gate of the
// user's conversation, behind a reservation that holds or waits for it.
func (c *Control) WaitGateEntrants(user, conversation string, count uint64) {
	c.t.Helper()
	c.do(controlproto.GateWaitingParams{User: user, Conversation: conversation, Count: count})
}

// A Hold is a hold that a control connection took on a flow.
type Hold struct {
	control *Control
	id      string
}

// Hold stops a flow from now on until the hold is released: one of the
// controlproto.Hold targets.
func (c *Control) Hold(target string) *Hold {
	c.t.Helper()
	return &Hold{control: c, id: c.id(c.do(controlproto.HoldParams{Target: target}))}
}

// Wait waits until count passes reached the hold, counting those that went away
// while they waited there.
func (h *Hold) Wait(count uint64) {
	h.control.t.Helper()
	h.control.do(controlproto.HoldWaitParams{Hold: h.id, Count: count})
}

// TryWait waits as Wait does and answers an error instead of failing the test,
// for a goroutine of the scenario: it answers one when the hold is released
// while it waits.
func (h *Hold) TryWait(count uint64) error {
	ctx, cancel := context.WithTimeout(context.Background(), Patience)
	defer cancel()
	_, err := h.control.call(ctx, controlproto.HoldWaitParams{Hold: h.id, Count: count})
	return err
}

// Release lets what waits go through, and everything that comes later.
func (h *Hold) Release() {
	h.control.t.Helper()
	h.control.do(controlproto.ReleaseParams{ID: h.id})
}

// RunRetention runs the user's retention pass at once and waits for its end.
func (c *Control) RunRetention(user string) {
	c.t.Helper()
	c.do(controlproto.RetentionRunParams{User: user})
}

// ObjectCounts reads what reached the object store since the backend started.
func (c *Control) ObjectCounts() controlproto.ObjectTally {
	c.t.Helper()
	tally, err := controlproto.DecodeResult[controlproto.ObjectTally](c.do(controlproto.ObjectsCountParams{}))
	if err != nil {
		c.t.Fatal(err)
	}
	return tally
}

// Mail lists the verification mail the backend sent, in order.
func (c *Control) Mail() []controlproto.Mail {
	c.t.Helper()
	list, err := controlproto.DecodeResult[controlproto.MailList](c.do(controlproto.MailListParams{}))
	if err != nil {
		c.t.Fatal(err)
	}
	return list.Mail
}

// FailMail makes the mail transport fail, or work again.
func (c *Control) FailMail(failing bool) {
	c.t.Helper()
	c.do(controlproto.MailFailParams{Failing: failing})
}
