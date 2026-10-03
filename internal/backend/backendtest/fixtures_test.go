package backendtest

import (
	"bufio"
	"context"
	"net"
	"reflect"
	"testing"
	"time"

	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(programTests{m}) }

// Validates all native fixture releases through the product's publisher. The
// workspace builds each named program once; no program or vendor is executed.
func TestNativePackagePublication(t *testing.T) {
	for _, program := range []string{"demi-file", "demi-browser", "demi-claude-code", "demi-native-fixture"} {
		t.Run(program, func(t *testing.T) {
			built, err := BuildPackage(t.Context(), t, program)
			if err != nil {
				t.Fatal(err)
			}
			catalog, err := built.Catalog(t.Context())
			if err != nil {
				t.Fatal(err)
			}
			again, err := built.Catalog(t.Context())
			if err != nil {
				t.Fatal(err)
			}
			if again != catalog {
				t.Fatal("publication was not shared")
			}
			if err := built.Descriptor.Validate(); err != nil {
				t.Fatal(err)
			}
			if len(built.Descriptor.Targets) != 1 {
				t.Fatal("fixture must describe the host target only")
			}
		})
	}
}

// The wire fixture runs over one local socket and uses no process or sleep.
// It checks responses, call observation and joining even with a client open.
func TestScriptedManagerProtocolAndClose(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancel()
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
	for i, call := range []machinewire.MachineCall{&machinewire.CurrentBaseVersion{}, &machinewire.Reconcile{}} {
		line, err := machinewire.EncodeLine(machinewire.MachineRequest{ID: string(rune('a' + i)), Call: call})
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
		decoded, err := machinewire.DecodeResponse(response)
		if err != nil {
			t.Fatal(err)
		}
		ok, success := decoded.(*machinewire.OK)
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

func (p programTests) Run() int { return programtest.Run(p.m) }
