//go:build linux

package machinemanager_test

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/machinemanager"
	"github.com/wspl/demi/internal/machinemanager/machinemanagertest"
	"github.com/wspl/demi/internal/machinemanagerproto"
)

type serverFixture struct {
	path    string
	service *machinemanagertest.Service
	deaths  chan machinemanagerproto.DeviceID
	stop    func()
}

func server(
	t *testing.T,
	script func(context.Context, machinemanagerproto.Call) (json.RawMessage, error),
) *serverFixture {
	t.Helper()
	ctx, cancel := context.WithCancel(t.Context())
	path := filepath.Join(t.TempDir(), "sock")
	socket, err := machinemanager.BindSocket(ctx, path)
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	service := &machinemanagertest.Service{Script: script}
	deaths := make(chan machinemanagerproto.DeviceID, 4)
	done := make(chan struct{})
	go func() {
		defer close(done)
		flight := machinemanager.Serve(ctx, socket, service, deaths)
		flight.Wait(context.Background())
	}()
	var once sync.Once
	stop := func() { once.Do(func() { cancel(); <-done }) }
	t.Cleanup(stop)
	return &serverFixture{path, service, deaths, stop}
}

type socketClient struct {
	conn   net.Conn
	reader *bufio.Reader
}

func (f *serverFixture) connect(t *testing.T) *socketClient {
	t.Helper()
	conn, err := net.Dial("unix", f.path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	return &socketClient{conn, bufio.NewReader(conn)}
}

func (c *socketClient) send(t *testing.T, line string) {
	t.Helper()
	if _, err := c.conn.Write([]byte(line)); err != nil {
		t.Fatal(err)
	}
}

func (c *socketClient) receive(t *testing.T) machinemanagerproto.MachineResponse {
	t.Helper()
	line, err := c.reader.ReadBytes('\n')
	if err != nil {
		t.Fatal(err)
	}
	value, err := machinemanagerproto.DecodeResponse(line[:len(line)-1])
	if err != nil {
		t.Fatal(err)
	}
	return value
}

func TestEveryCallReachesService(t *testing.T) {
	f := server(t, func(_ context.Context, call machinemanagerproto.Call) (json.RawMessage, error) {
		switch call.(type) {
		case *machinemanagerproto.CurrentBaseVersion:
			return machinemanagerproto.BaseVersion("base").MarshalJSON()
		case *machinemanagerproto.RuntimeStateCall:
			return machinemanagerproto.RuntimeStateStopped.MarshalJSON()
		case *machinemanagerproto.Reconcile,
			*machinemanagerproto.ImageState,
			*machinemanagerproto.Wake,
			*machinemanagerproto.Hibernate,
			*machinemanagerproto.Checkpoint,
			*machinemanagerproto.GrowVolume,
			*machinemanagerproto.Reset:
			return json.RawMessage("null"), nil
		}
		return nil, errors.New("unknown call")
	})
	c := f.connect(t)
	lines := []string{
		`{"id":"1","op":"reconcile","params":{}}`,
		`{"id":"2","op":"current_base_version","params":{}}`,
		`{"id":"3","op":"image_state","params":{"deviceId":"dev-1"}}`,
		`{"id":"4","op":"runtime_state","params":{"deviceId":"dev-1"}}`,
		`{"id":"5","op":"wake","params":{"deviceId":"dev-1","boot":{"backendUrl":"http://backend","deviceToken":"tok"}}}`,
		`{"id":"6","op":"checkpoint","params":{"deviceId":"dev-1"}}`,
		`{"id":"7","op":"grow_volume","params":{"deviceId":"dev-1","volume":"home","bytes":4096}}`,
		`{"id":"8","op":"reset","params":{"deviceId":"dev-1","operationId":"op-1","baseVersion":"base-2"}}`,
		`{"id":"9","op":"hibernate","params":{"deviceId":"dev-1"}}`,
	}
	for _, line := range lines {
		request, err := machinemanagerproto.DecodeRequest([]byte(line))
		if err != nil {
			t.Fatal(err)
		}
		c.send(t, line+"\n")
		response, ok := c.receive(t).(*machinemanagerproto.OK)
		if !ok || response.ID != request.ID {
			t.Fatalf("reply: %+v", response)
		}
		want := "null"
		if request.ID == "2" {
			want = `"base"`
		}
		if request.ID == "4" {
			want = `"stopped"`
		}
		if string(response.Result) != want {
			t.Fatalf("result %s", response.Result)
		}
	}
	f.stop()
	calls := f.service.Calls()
	if len(calls) != len(lines) {
		t.Fatal(len(calls))
	}
	for i, line := range lines {
		request, _ := machinemanagerproto.DecodeRequest([]byte(line))
		if !reflect.DeepEqual(request.Call, calls[i]) {
			t.Fatal(calls)
		}
	}
}

func TestFailureLeavesConnectionUsable(t *testing.T) {
	f := server(t, func(_ context.Context, call machinemanagerproto.Call) (json.RawMessage, error) {
		if call.Name() == "hibernate" {
			return nil, errors.New("no such machine")
		}
		return json.RawMessage("null"), nil
	})
	c := f.connect(t)
	c.send(t, "{\"id\":\"1\",\"op\":\"hibernate\",\"params\":{\"deviceId\":\"dev-9\"}}\n")
	response, ok := c.receive(t).(*machinemanagerproto.ErrorResponse)
	if !ok || response.ID != "1" || response.Message != "no such machine" {
		t.Fatalf("%+v", response)
	}
	c.send(t, "\n{\"id\":\"2\",\"op\":\"reconcile\",\"params\":{}}\n")
	if reply, ok := c.receive(t).(*machinemanagerproto.OK); !ok || reply.ID != "2" {
		t.Fatalf("%+v", reply)
	}
}

func TestConcurrentRepliesCompleteInOrder(t *testing.T) {
	release := make(chan struct{})
	var once sync.Once
	unblock := func() { once.Do(func() { close(release) }) }
	f := server(t, func(_ context.Context, call machinemanagerproto.Call) (json.RawMessage, error) {
		if call.Name() == "current_base_version" {
			<-release
		}
		return json.RawMessage("null"), nil
	})
	t.Cleanup(unblock)
	c := f.connect(t)
	c.send(
		t,
		"{\"id\":\"slow\",\"op\":\"current_base_version\",\"params\":{}}\n"+
			"{\"id\":\"fast\",\"op\":\"reconcile\",\"params\":{}}\n",
	)
	if reply, ok := c.receive(t).(*machinemanagerproto.OK); !ok || reply.ID != "fast" {
		t.Fatalf("%+v", reply)
	}
	unblock()
	if reply, ok := c.receive(t).(*machinemanagerproto.OK); !ok || reply.ID != "slow" {
		t.Fatalf("%+v", reply)
	}
}

func TestDeathReachesEveryConnection(t *testing.T) {
	f := server(t, nil)
	clients := []*socketClient{f.connect(t), f.connect(t)}
	for _, c := range clients {
		c.send(t, "{\"id\":\"1\",\"op\":\"reconcile\",\"params\":{}}\n")
		c.receive(t)
	}
	f.deaths <- machinemanagerproto.DeviceID("dev-1")
	for _, c := range clients {
		death, ok := c.receive(t).(*machinemanagerproto.Death)
		if !ok || death.DeviceID != "dev-1" {
			t.Fatalf("%+v", death)
		}
	}
}

func TestBadLineDropsConnectionAndLaterLines(t *testing.T) {
	f := server(t, nil)
	for _, bad := range []string{
		"{\"id\":\"1\",\"op\":\"wake\",\"params\":{}}\n",
		"not json\n",
		string([]byte{255, '\n'}),
		strings.Repeat("x", machinemanagerproto.MaxLineBytes+1) + "\n",
	} {
		c := f.connect(t)
		_, _ = c.conn.Write([]byte(bad + "{\"id\":\"2\",\"op\":\"reconcile\",\"params\":{}}\n"))
		// The read ends only when the server closes the connection.
		if line, err := c.reader.ReadBytes('\n'); err == nil {
			t.Fatalf("bad frame answered: %s", line)
		}
	}
	f.stop()
	if len(f.service.Calls()) != 0 {
		t.Fatal("served behind bad frame")
	}
}

func TestDisconnectedRequestCompletes(t *testing.T) {
	entered := make(chan struct{})
	release := make(chan struct{})
	finished := make(chan struct{})
	var once sync.Once
	unblock := func() { once.Do(func() { close(release) }) }
	f := server(t, func(_ context.Context, _ machinemanagerproto.Call) (json.RawMessage, error) {
		close(entered)
		<-release
		close(finished)
		return json.RawMessage("null"), nil
	})
	t.Cleanup(unblock)
	c := f.connect(t)
	c.send(t, "{\"id\":\"1\",\"op\":\"current_base_version\",\"params\":{}}\n")
	<-entered
	if err := c.conn.Close(); err != nil {
		t.Fatal(err)
	}
	unblock()
	f.stop()
	<-finished
}

func TestSocketOwnership(t *testing.T) {
	path := filepath.Join(t.TempDir(), "socket")
	old, err := net.ListenUnix("unix", &net.UnixAddr{Name: path, Net: "unix"})
	if err != nil {
		t.Fatal(err)
	}
	old.SetUnlinkOnClose(false)
	if err = old.Close(); err != nil {
		t.Fatal(err)
	}
	socket, err := machinemanager.BindSocket(t.Context(), path)
	if err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o660 {
		t.Fatal(info.Mode())
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	machinemanager.Serve(ctx, socket, &machinemanagertest.Service{}, nil).Wait(context.Background())
	if _, err = os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("socket retained: %v", err)
	}
	if err = os.WriteFile(path, []byte("data"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err = machinemanager.BindSocket(t.Context(), path); err == nil {
		t.Fatal("ordinary file replaced")
	}
	data, err := os.ReadFile(path)
	if err != nil || string(data) != "data" {
		t.Fatalf("file: %s %v", data, err)
	}
}

func TestSecondManagerRefusedWithPath(t *testing.T) {
	data, runtime := t.TempDir(), t.TempDir()
	first, err := machinemanager.AcquireLock(data, runtime)
	if err != nil {
		t.Fatal(err)
	}
	_, err = machinemanager.AcquireLock(data, runtime)
	if err == nil || err.Error() != "Another Cloud manager owns "+filepath.Join(data, "manager.lock") {
		t.Fatal(err)
	}
	if err = first.Close(); err != nil {
		t.Fatal(err)
	}
	next, err := machinemanager.AcquireLock(data, runtime)
	if err != nil {
		t.Fatal(err)
	}
	if err = next.Close(); err != nil {
		t.Fatal(err)
	}
}
