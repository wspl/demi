package backendtest

import (
	"bufio"
	"net"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/machinemanagerproto"
	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

// TestMain checks that fixture tests release their goroutines and shared program builds.
func TestMain(m *testing.M) {
	goleak.VerifyTestMain(programTests{m})
}

// The wire fixture runs over one local socket and uses no process or sleep.
// It checks responses, call observation and joining even with a client open.
func TestScriptedManagerProtocolAndClose(t *testing.T) {
	ctx := t.Context()
	manager, err := StartScriptedManager(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	socket, err := (&net.Dialer{}).DialContext(ctx, "unix", manager.Socket())
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := socket.Close(); err != nil {
			t.Error(err)
		}
	}()
	reader := bufio.NewReader(socket)
	calls := []machinemanagerproto.Call{&machinemanagerproto.CurrentBaseVersion{}, &machinemanagerproto.Reconcile{}}
	for i, call := range calls {
		line, err := machinemanagerproto.EncodeLine(
			machinemanagerproto.MachineRequest{ID: string(rune('a' + i)), Call: call},
		)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := socket.Write(line); err != nil {
			t.Fatal(err)
		}
		response, err := reader.ReadBytes('\n')
		if err != nil {
			t.Fatal(err)
		}
		decoded, err := machinemanagerproto.DecodeResponse(response)
		if err != nil {
			t.Fatal(err)
		}
		ok, success := decoded.(*machinemanagerproto.OK)
		if !success {
			t.Fatalf("manager refused: %v", decoded)
		}
		if i == 0 && string(ok.Result) != `"test-base"` {
			t.Fatalf("base: %s", ok.Result)
		}
	}
	if _, err := manager.Arrival(ctx, "reconcile"); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(manager.Calls(), []string{"reconcile"}) {
		t.Fatalf("calls: %v", manager.Calls())
	}
	if err := manager.Close(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := reader.ReadByte(); err == nil {
		t.Fatal("manager left connection open")
	}
}

// programTests releases the backend scenarios' shared program builds before leak checking.
type programTests struct{ m *testing.M }

// Run releases shared program builds before leak checking.
func (p programTests) Run() int {
	return programtest.Run(p.m)
}
