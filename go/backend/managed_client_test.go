package backend_test

import (
	"bufio"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/machinesproto"
)

// managerSocket is a listening socket in a short directory of its own: a
// socket's path is at most about a hundred bytes.
func managerSocket(t *testing.T) (string, *net.UnixListener) {
	t.Helper()
	directory := must(os.MkdirTemp("", "machines"))
	t.Cleanup(func() { os.RemoveAll(directory) })
	path := filepath.Join(directory, "machines.sock")
	listener := must(net.ListenUnix("unix", &net.UnixAddr{Name: path, Net: "unix"}))
	t.Cleanup(func() { listener.Close() })
	return path, listener
}

// connected says whether a client has connected to listener, without
// waiting for one.
func connected(t *testing.T, listener *net.UnixListener) bool {
	t.Helper()
	if err := listener.SetDeadline(time.Now()); err != nil {
		t.Fatal(err)
	}
	defer listener.SetDeadline(time.Time{})
	conn, err := listener.Accept()
	if err == nil {
		conn.Close()
		return true
	}
	if !errors.Is(err, os.ErrDeadlineExceeded) {
		t.Fatal(err)
	}
	return false
}

// managerPeer is one connection of a machine manager the test scripts.
type managerPeer struct {
	conn  net.Conn
	lines *bufio.Scanner
}

func acceptManager(t *testing.T, listener *net.UnixListener) *managerPeer {
	t.Helper()
	conn := must(listener.Accept())
	t.Cleanup(func() { conn.Close() })
	return &managerPeer{conn: conn, lines: bufio.NewScanner(conn)}
}

func (p *managerPeer) request(t *testing.T) machinesproto.Request {
	t.Helper()
	if !p.lines.Scan() {
		t.Fatalf("no request: %v", p.lines.Err())
	}
	return must(machinesproto.DecodeRequest(p.lines.Bytes()))
}

func (p *managerPeer) write(t *testing.T, response machinesproto.Response) {
	t.Helper()
	p.writeRaw(t, string(must(machinesproto.EncodeResponse(response))))
}

func (p *managerPeer) writeRaw(t *testing.T, lines string) {
	t.Helper()
	if _, err := p.conn.Write([]byte(lines)); err != nil {
		t.Fatal(err)
	}
}

func (p *managerPeer) ok(t *testing.T, id, result string) {
	t.Helper()
	p.write(t, machinesproto.OK{ID: id, Result: jsontext.Value(result)})
}

// closed says whether the client closed the connection.
func (p *managerPeer) closed() bool { return !p.lines.Scan() && p.lines.Err() == nil }

// Calls cross the socket, which the first call opens, and each reply
// reaches its call whatever order the replies come in (managed-hosts.md §
// Control and ownership).
// Cost: a Unix socket.
func TestCallsCrossTheSocketAndTheirRepliesAreMatchedByIDWhateverTheirOrder(t *testing.T) {
	path, listener := managerSocket(t)
	client, _ := backend.NewMachinesClient(path)
	if connected(t, listener) {
		t.Fatal("the client connected before its first call")
	}
	// Four devices' image states and the base version, asked at once.
	devices := []machinesproto.DeviceID{"dev-1", "dev-2", "dev-3", "dev-4"}
	failures := make(chan error, len(devices)+1)
	var calls sync.WaitGroup
	for _, device := range devices {
		calls.Go(func() {
			image, err := client.ImageState(t.Context(), device)
			if err == nil && (image == nil || string(image.Generation) != "gen-"+string(device)) {
				err = fmt.Errorf("%s answered %+v", device, image)
			}
			failures <- err
		})
	}
	calls.Go(func() {
		base, err := client.CurrentBaseVersion(t.Context())
		if err == nil && base != "base-1" {
			err = fmt.Errorf("the base version is %q", base)
		}
		failures <- err
	})
	peer := acceptManager(t, listener)
	var requests []machinesproto.Request
	for range len(devices) + 1 {
		requests = append(requests, peer.request(t))
	}
	ids := make([]string, 0, len(requests))
	for _, request := range requests {
		ids = append(ids, request.ID)
	}
	slices.Sort(ids)
	if !slices.Equal(ids, []string{"1", "2", "3", "4", "5"}) {
		t.Fatalf("ids %v", ids)
	}
	// The last request is answered first.
	for _, request := range slices.Backward(requests) {
		switch call := request.Call.(type) {
		case machinesproto.ImageStateParams:
			peer.ok(t, request.ID, fmt.Sprintf(`{"generation":"gen-%s","baseVersion":"base-1","resetId":null,"systemBytes":1024,"homeBytes":2048}`, call.DeviceID))
		case machinesproto.CurrentBaseVersionParams:
			peer.ok(t, request.ID, `"base-1"`)
		default:
			t.Fatalf("asked %+v", request)
		}
	}
	calls.Wait()
	close(failures)
	for err := range failures {
		if err != nil {
			t.Error(err)
		}
	}
}

// A failure reply fails its call and the connection stays usable; lines
// that are not replies, or answer nothing asked, are dropped; and a result
// the operation does not have fails its call.
// Cost: a Unix socket.
func TestAFailureReplyFailsItsCallAndTheConnectionStaysUsable(t *testing.T) {
	path, listener := managerSocket(t)
	client, _ := backend.NewMachinesClient(path)
	failed := make(chan error)
	go func() { failed <- client.Hibernate(t.Context(), "dev-9") }()
	peer := acceptManager(t, listener)
	request := peer.request(t)
	peer.write(t, machinesproto.Failure{ID: request.ID, Message: "no such machine"})
	var refused *backend.MachinesError
	if err := <-failed; !errors.As(err, &refused) || refused.Kind != backend.MachinesFailed || err.Error() != "no such machine" {
		t.Fatalf("hibernate: %v", err)
	}
	peer.writeRaw(t, "not json\n{\"type\":\"ok\",\"id\":\"99\",\"result\":null}\n\n")
	answered := make(chan machinesproto.BaseVersion)
	go func() { answered <- must(client.CurrentBaseVersion(t.Context())) }()
	peer.ok(t, peer.request(t).ID, `"base-1"`)
	if base := <-answered; base != "base-1" {
		t.Fatalf("base %q", base)
	}
	odd := make(chan error)
	go func() {
		_, err := client.CurrentBaseVersion(t.Context())
		odd <- err
	}()
	peer.ok(t, peer.request(t).ID, `7`)
	if err := <-odd; !errors.As(err, &refused) || refused.Kind != backend.MachinesResult {
		t.Fatalf("an odd result: %v", err)
	}
}

// A death reaches the router; closing reconciles over the open connection
// and disconnects until the next call, and a client that never connected
// closes without a word.
// Cost: a Unix socket.
func TestADeathReachesTheRouterAndCloseReconcilesThenDisconnectsUntilTheNextCall(t *testing.T) {
	path, listener := managerSocket(t)
	client, deaths := backend.NewMachinesClient(path)
	if err := client.Close(t.Context()); err != nil || connected(t, listener) {
		t.Fatalf("closing a client that never connected: %v", err)
	}
	first := make(chan error)
	go func() {
		_, err := client.CurrentBaseVersion(t.Context())
		first <- err
	}()
	peer := acceptManager(t, listener)
	request := peer.request(t)
	peer.write(t, machinesproto.Death{DeviceID: "dev-1"})
	peer.ok(t, request.ID, `"base-1"`)
	if err := <-first; err != nil {
		t.Fatal(err)
	}
	if device := <-deaths; device != "dev-1" {
		t.Fatalf("death of %q", device)
	}

	closing := make(chan error)
	go func() { closing <- client.Close(t.Context()) }()
	reconcile := peer.request(t)
	if _, ok := reconcile.Call.(machinesproto.ReconcileParams); !ok {
		t.Fatalf("closing sent %+v", reconcile)
	}
	peer.ok(t, reconcile.ID, "null")
	if err := <-closing; err != nil {
		t.Fatal(err)
	}
	if !peer.closed() {
		t.Fatal("the client kept its connection")
	}

	again := make(chan machinesproto.BaseVersion)
	go func() { again <- must(client.CurrentBaseVersion(t.Context())) }()
	peer = acceptManager(t, listener)
	peer.ok(t, peer.request(t).ID, `"base-2"`)
	if base := <-again; base != "base-2" {
		t.Fatalf("base %q", base)
	}
}

// A manager that goes away fails the calls in flight as unavailable; with
// no manager listening a call fails at once; and a manager that returns is
// dialed again.
// Cost: a Unix socket.
func TestAManagerThatGoesAwayFailsTheCallsInFlightAndIsDialedAgain(t *testing.T) {
	path, listener := managerSocket(t)
	client, _ := backend.NewMachinesClient(path)
	inFlight := make(chan error)
	go func() {
		_, err := client.CurrentBaseVersion(t.Context())
		inFlight <- err
	}()
	peer := acceptManager(t, listener)
	peer.request(t)
	peer.conn.Close()
	listener.Close()
	if err := <-inFlight; err == nil || !strings.HasPrefix(err.Error(), "Machine manager unavailable during current_base_version") {
		t.Fatalf("in flight: %v", err)
	}
	var refused *backend.MachinesError
	if _, err := client.CurrentBaseVersion(t.Context()); !errors.As(err, &refused) || refused.Kind != backend.MachinesUnavailable {
		t.Fatalf("without a manager: %v", err)
	}

	listener = must(net.ListenUnix("unix", &net.UnixAddr{Name: path, Net: "unix"}))
	t.Cleanup(func() { listener.Close() })
	later := make(chan machinesproto.BaseVersion)
	go func() { later <- must(client.CurrentBaseVersion(t.Context())) }()
	peer = acceptManager(t, listener)
	peer.ok(t, peer.request(t).ID, `"base-1"`)
	if base := <-later; base != "base-1" {
		t.Fatalf("base %q", base)
	}
}
