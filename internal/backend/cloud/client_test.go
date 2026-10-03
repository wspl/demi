package cloud

import (
	"bufio"
	"context"
	"errors"
	"io"
	"net"
	"path/filepath"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/webapi"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

type managerPeer struct {
	t      *testing.T
	conn   net.Conn
	reader *bufio.Reader
}

// newPeer owns one scripted manager connection and its JSON line reader.
func newPeer(t *testing.T, conn net.Conn) *managerPeer {
	t.Helper()
	t.Cleanup(func() { _ = conn.Close() })
	return &managerPeer{t: t, conn: conn, reader: bufio.NewReader(conn)}
}

func (p *managerPeer) request() machinewire.MachineRequest {
	p.t.Helper()
	line, err := p.reader.ReadBytes('\n')
	if err != nil {
		p.t.Fatal(err)
	}
	request, err := machinewire.DecodeRequest(line[:len(line)-1])
	if err != nil {
		p.t.Fatal(err)
	}
	return request
}

func (p *managerPeer) write(response machinewire.MachineResponse) {
	p.t.Helper()
	line, err := machinewire.EncodeLine(response)
	if err != nil {
		p.t.Fatal(err)
	}
	if _, err = p.conn.Write(line); err != nil {
		p.t.Fatal(err)
	}
}

func (p *managerPeer) ok(id, result string) {
	p.t.Helper()
	p.write(&machinewire.OK{ID: id, Result: []byte(result)})
}

func (p *managerPeer) closed() {
	p.t.Helper()
	_, err := p.reader.ReadByte()
	if !errors.Is(err, io.EOF) {
		p.t.Fatalf("connection not closed: %v", err)
	}
}

// scriptedClient replaces only dialing; real manager framing and dispatch run inside the bubble.
func scriptedClient(t *testing.T) (*Client, <-chan webapi.DeviceID, <-chan net.Conn) {
	t.Helper()
	c, deaths := NewClient(t.Context(), "scripted")
	connections := make(chan net.Conn)
	c.dial = func(ctx context.Context, _, _ string) (net.Conn, error) {
		local, peer := net.Pipe()
		select {
		case connections <- peer:
			return local, nil
		case <-ctx.Done():
			_ = local.Close()
			_ = peer.Close()
			return nil, ctx.Err()
		}
	}
	t.Cleanup(func() {
		c.cancel()
		<-c.done
	})
	return c, deaths, connections
}

type managerResult[T any] struct {
	value T
	err   error
}

// managerCall starts one caller whose completion the test receives and joins.
func managerCall[T any](ctx context.Context, c *Client, params machinewire.Operation[T]) <-chan managerResult[T] {
	result := make(chan managerResult[T], 1)
	go func() {
		v, err := Call(ctx, c, params)
		result <- managerResult[T]{v, err}
	}()
	return result
}

func TestClientMatchesRepliesByID(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		c, _, connections := scriptedClient(t)
		synctest.Wait()
		select {
		case <-connections:
			t.Fatal("dialed before first call")
		default:
		}
		base := managerCall(t.Context(), c, machinewire.CurrentBaseVersionParams{})
		image := managerCall(t.Context(), c, machinewire.ImageStateParams{DeviceID: "dev-1"})
		peer := newPeer(t, <-connections)
		first, second := peer.request(), peer.request()
		if first.ID != "1" || second.ID != "2" {
			t.Fatalf("request IDs: %s, %s", first.ID, second.ID)
		}
		for _, request := range []machinewire.MachineRequest{second, first} {
			if request.Call.Name() == "current_base_version" {
				peer.ok(request.ID, `"base-1"`)
			} else {
				peer.ok(
					request.ID,
					`{"generation":"gen-1","baseVersion":"base-1","resetId":null,"systemBytes":1024,"homeBytes":2048}`,
				)
			}
		}
		b, i := <-base, <-image
		if b.err != nil || b.value != "base-1" {
			t.Fatalf("base: %+v", b)
		}
		if i.err != nil || i.value == nil || i.value.Generation != "gen-1" || i.value.BaseVersion != "base-1" ||
			i.value.ResetID != nil ||
			i.value.SystemBytes != 1024 ||
			i.value.HomeBytes != 2048 {
			t.Fatalf("image: %+v", i)
		}
	})
}

func TestClientFailureKeepsConnectionUsable(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		c, _, connections := scriptedClient(t)
		failing := managerCall(t.Context(), c, machinewire.HibernateParams{DeviceID: "dev-9"})
		peer := newPeer(t, <-connections)
		request := peer.request()
		peer.write(&machinewire.ErrorResponse{ID: request.ID, Message: "no such machine"})
		var failure *ManagerError
		if err := (<-failing).err; !errors.As(err, &failure) || failure.Kind != ManagerFailed ||
			err.Error() != "no such machine" {
			t.Fatalf("failure: %v", err)
		}
		if _, err := peer.conn.Write(
			[]byte("not json\n{\"type\":\"ok\",\"id\":\"99\",\"result\":null}\n\n"),
		); err != nil {
			t.Fatal(err)
		}
		answered := managerCall(t.Context(), c, machinewire.CurrentBaseVersionParams{})
		request = peer.request()
		peer.ok(request.ID, `"base-1"`)
		if result := <-answered; result.err != nil || result.value != "base-1" {
			t.Fatalf("result: %+v", result)
		}
		odd := managerCall(t.Context(), c, machinewire.CurrentBaseVersionParams{})
		peer.ok(peer.request().ID, "7")
		if err := (<-odd).err; !errors.As(err, &failure) || failure.Kind != ManagerResult {
			t.Fatalf("wrong result: %v", err)
		}
	})
}

func TestClientDeathAndDisconnect(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		c, deaths, connections := scriptedClient(t)
		if err := c.Disconnect(t.Context()); err != nil {
			t.Fatal(err)
		}
		first := managerCall(t.Context(), c, machinewire.CurrentBaseVersionParams{})
		peer := newPeer(t, <-connections)
		request := peer.request()
		peer.write(&machinewire.Death{DeviceID: "dev-1"})
		peer.ok(request.ID, `"base-1"`)
		if result := <-first; result.err != nil {
			t.Fatal(result.err)
		}
		if device := <-deaths; device != "dev-1" {
			t.Fatal(device)
		}
		closed := make(chan error, 1)
		go func() { closed <- c.Disconnect(t.Context()) }()
		reconcile := peer.request()
		if reconcile.Call.Name() != "reconcile" {
			t.Fatal(reconcile.Call.Name())
		}
		peer.ok(reconcile.ID, "null")
		if err := <-closed; err != nil {
			t.Fatal(err)
		}
		peer.closed()
		again := managerCall(t.Context(), c, machinewire.CurrentBaseVersionParams{})
		peer = newPeer(t, <-connections)
		request = peer.request()
		if request.Call.Name() != "current_base_version" {
			t.Fatal("reconnect unexpectedly reconciled")
		}
		peer.ok(request.ID, `"base-2"`)
		if result := <-again; result.err != nil || result.value != "base-2" {
			t.Fatalf("reconnected: %+v", result)
		}
		go func() { closed <- c.Close(t.Context()) }()
		request = peer.request()
		if request.Call.Name() != "reconcile" {
			t.Fatal(request.Call.Name())
		}
		peer.ok(request.ID, "null")
		if err := <-closed; err != nil {
			t.Fatal(err)
		}
		peer.closed()
		if _, ok := <-deaths; ok {
			t.Fatal("death stream still open")
		}
		if err := c.Close(t.Context()); err != nil {
			t.Fatal(err)
		}
		if _, err := Call(t.Context(), c, machinewire.CurrentBaseVersionParams{}); !errors.As(err, new(*ManagerError)) {
			t.Fatalf("call after close: %v", err)
		}
	})
}

func TestClientDropFailsPendingAndReconnects(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		c, _, connections := scriptedClient(t)
		first := managerCall(t.Context(), c, machinewire.CurrentBaseVersionParams{})
		second := managerCall(t.Context(), c, machinewire.CurrentBaseVersionParams{})
		peer := newPeer(t, <-connections)
		peer.request()
		peer.request()
		if err := peer.conn.Close(); err != nil {
			t.Fatal(err)
		}
		for _, result := range []managerResult[machinewire.BaseVersion]{<-first, <-second} {
			var unavailable *ManagerError
			if !errors.As(result.err, &unavailable) || unavailable.Kind != ManagerUnavailable ||
				unavailable.Operation != "current_base_version" {
				t.Fatalf("drop: %v", result.err)
			}
		}
		// The next dial can fail; neither an old call nor this refusal is replayed.
		dial := c.dial
		c.dial = func(context.Context, string, string) (net.Conn, error) { return nil, io.ErrClosedPipe }
		var unavailable *ManagerError
		if _, err := Call(
			t.Context(),
			c,
			machinewire.CurrentBaseVersionParams{},
		); !errors.Is(err, io.ErrClosedPipe) || !errors.As(err, &unavailable) ||
			unavailable.Kind != ManagerUnavailable {
			t.Fatalf("dial failure: %v", err)
		}
		c.dial = dial
		later := managerCall(t.Context(), c, machinewire.CurrentBaseVersionParams{})
		peer = newPeer(t, <-connections)
		request := peer.request()
		peer.ok(request.ID, `"base-1"`)
		if result := <-later; result.err != nil || result.value != "base-1" {
			t.Fatalf("later: %+v", result)
		}
	})
}

func TestClientCanceledCallerAndCloseDuringBlockedWrite(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		c, _, connections := scriptedClient(t)
		ctx, cancel := context.WithCancel(t.Context())
		pending := managerCall(ctx, c, machinewire.CurrentBaseVersionParams{})
		peer := newPeer(t, <-connections)
		synctest.Wait() // The supervisor is blocked writing to the unread pipe.
		cancel()
		if result := <-pending; !errors.Is(result.err, context.Canceled) {
			t.Fatal(result.err)
		}
		cleanup, end := context.WithCancel(t.Context())
		end()
		if err := c.Close(cleanup); !errors.Is(err, context.Canceled) {
			t.Fatalf("close: %v", err)
		}
		peer.closed()
	})
}

// The socket smoke test costs no timer waits and verifies the production Unix dial path.
func TestClientUnixSocket(t *testing.T) {
	path := filepath.Join(t.TempDir(), "manager.sock")
	listener, err := net.Listen("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := listener.Close(); err != nil {
			t.Error(err)
		}
	}()
	c, _ := NewClient(t.Context(), path)
	defer func() {
		c.cancel()
		<-c.done
	}()
	var workers sync.WaitGroup
	workers.Go(func() {
		conn, err := listener.Accept()
		if err != nil {
			t.Error(err)
			return
		}
		defer func() { _ = conn.Close() }() // Closing the scripted connection only releases its descriptor.
		scanner := bufio.NewScanner(conn)
		if !scanner.Scan() {
			t.Error("no request")
			return
		}
		request, err := machinewire.DecodeRequest(scanner.Bytes())
		if err != nil {
			t.Error(err)
			return
		}
		line, err := machinewire.EncodeLine(&machinewire.OK{ID: request.ID, Result: []byte(`"socket-base"`)})
		if err != nil {
			t.Error(err)
			return
		}
		if _, err := conn.Write(line); err != nil {
			t.Error(err)
		}
	})
	result, err := Call(t.Context(), c, machinewire.CurrentBaseVersionParams{})
	workers.Wait()
	if err != nil || result != "socket-base" {
		t.Fatalf("socket call: %s, %v", result, err)
	}
}
