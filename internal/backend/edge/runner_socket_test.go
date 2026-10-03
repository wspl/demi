package edge

import (
	"context"
	"errors"
	"net/netip"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/backend/usershard/usershardtest"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// lookupHooks observes the held lookup ending, so the test need not release it
// (or shut down the edge) to make a disconnected runner's handler finish.
type lookupHooks struct {
	usershardtest.Hooks
	exited chan error
}

func (h *lookupHooks) Hello(ctx context.Context, step usershard.HelloStep) error {
	err := h.Hooks.Hello(ctx, step)
	if step == usershard.HelloTokenLookup {
		h.exited <- err
	}
	return err
}

// A real listener, SQLite and shard exercise the pre-adoption boundary. No model
// calls or sleeps; all waits use protocol events. Expected cost: under one second.
func TestRunnerCloseDuringHelloTokenLookupNeverBecomesOnline(t *testing.T) {
	services := usershardtest.StartServices(t)
	hooks := &lookupHooks{exited: make(chan error, 1)}
	services.Hooks = hooks
	hold := hooks.Hellos.Hold(t, usershard.HelloTokenLookup)
	shards := usershardtest.StartShards(t, services)
	device, hello := pairedRunner(t, services)
	socket := dialRunner(t, services, shards)
	writeRunnerHello(t, socket, hello)
	if err := hold.UntilArrived(t.Context(), 1); err != nil {
		t.Fatal(err)
	}
	if err := socket.Close(websocket.StatusNormalClosure, ""); err != nil {
		t.Fatal(err)
	}
	// A Close frame must cancel lookup without releasing its test hold.
	if err := <-hooks.exited; !errors.Is(err, context.Canceled) {
		t.Fatalf("lookup was not canceled: %v", err)
	}
	shard, err := shards.Of(t.Context(), device.User)
	if err != nil {
		t.Fatal(err)
	}
	devices, err := shard.DeviceList(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if len(devices) != 1 || devices[0].Online || devices[0].LastSeenAt != nil {
		t.Fatalf("closed runner was adopted: %#v", devices)
	}
}

func TestRunnerRepeatedHelloBeforeAcceptanceIsDropped(t *testing.T) {
	for _, phase := range []string{"lookup", "claim"} {
		t.Run(phase, func(t *testing.T) {
			services := usershardtest.StartServices(t)
			hooks := &usershardtest.Hooks{}
			services.Hooks = hooks
			hold := hooks.Hellos.Hold(t, usershard.HelloTokenLookup)
			shards := usershardtest.StartShards(t, services)
			device, hello := pairedRunner(t, services)
			token := *hello.DeviceToken
			if phase == "claim" {
				hello.DeviceToken = nil
			}
			socket := dialRunner(t, services, shards)
			writeRunnerHello(t, socket, hello)
			var pending *runners.PendingRunner
			if phase == "lookup" {
				if err := hold.UntilArrived(t.Context(), 1); err != nil {
					t.Fatal(err)
				}
			} else {
				_, data, err := socket.Read(t.Context())
				if err != nil {
					t.Fatal(err)
				}
				message, err := runnerwire.DecodeInbound(data)
				if err != nil {
					t.Fatal(err)
				}
				claim, ok := message.(*runnerwire.ClaimPending)
				if !ok {
					t.Fatalf("claim answer: %#v", message)
				}
				code, ok := runners.ParseClaimCode(claim.ClaimToken)
				if !ok {
					t.Fatal("invalid claim code")
				}
				pending = services.Claims.Take(code)
				if pending == nil {
					t.Fatal("claim not registered")
				}
				defer pending.Release()
			}
			writeRunnerHello(t, socket, hello)
			// Rust drops every frame here, including nonprotocol text and malformed data.
			if err := socket.Write(
				t.Context(),
				websocket.MessageText,
				[]byte("ignored before acceptance"),
			); err != nil {
				t.Fatal(err)
			}
			if err := socket.Write(t.Context(), websocket.MessageBinary, []byte{0xc1}); err != nil {
				t.Fatal(err)
			}
			answer := make(chan runnerwire.Inbound, 1)
			readDone := make(chan struct{})
			readCtx, cancelRead := context.WithCancel(t.Context())
			defer func() {
				cancelRead()
				<-readDone
			}()
			go func() {
				defer close(readDone)
				_, data, err := socket.Read(readCtx)
				if err != nil {
					answer <- nil
					return
				}
				message, err := runnerwire.DecodeInbound(data)
				if err != nil {
					answer <- nil
					return
				}
				answer <- message
			}()
			// Pong proves the lifetime reader consumed the preceding frames while the
			// acceptance was held. The peer reader above handles the WebSocket control frame.
			if err := socket.Ping(t.Context()); err != nil {
				t.Fatal(err)
			}
			if phase == "lookup" {
				hold.Release()
				message := <-answer
				accepted, ok := message.(*runnerwire.HelloOK)
				if !ok || accepted.DeviceID != string(device.ID) {
					t.Fatalf("hello answer: %#v", message)
				}
			} else {
				type grantResult struct {
					device webapi.DeviceDTO
					err    error
				}
				granted := make(chan grantResult, 1)
				go func() {
					bound, err := pending.Grant(t.Context(), device, token)
					granted <- grantResult{bound, err}
				}()
				message := <-answer
				claimed, ok := message.(*runnerwire.Claimed)
				if !ok || claimed.DeviceToken != token {
					t.Fatal("runner did not receive its claimed token")
				}
				result := <-granted
				if result.err != nil || !result.device.Online {
					t.Fatalf("claim did not bind: %#v, %v", result.device, result.err)
				}
			}
			shard, err := shards.Of(t.Context(), device.User)
			if err != nil {
				t.Fatal(err)
			}
			devices, err := shard.DeviceList(t.Context())
			if err != nil || len(devices) != 1 || !devices[0].Online {
				t.Fatalf("runner not online after acceptance: %#v, %v", devices, err)
			}
		})
	}
}

func pairedRunner(t *testing.T, services *usershard.Services) (database.DeviceRecord, *runnerwire.Hello) {
	t.Helper()
	user := databasetest.Master(t.Context(), t, services.Control)
	token := runners.NewDeviceToken()
	device, err := services.Control.CreateDevice(
		t.Context(),
		user.ID,
		"fixture",
		runnerwire.RunnerPlatformLinux,
		database.HashToken(token.Expose()),
	)
	if err != nil {
		t.Fatal(err)
	}
	return device, &runnerwire.Hello{
		Protocol:    runnerwire.Version,
		DeviceToken: &token,
		Runner: runnerwire.RunnerInfo{
			Name:     "fixture",
			Platform: runnerwire.RunnerPlatformLinux,
			Version:  "fixture",
			Identity: runnerwire.HostIdentity{Hostname: "fixture", HomeDir: "/home/fixture"},
		},
	}
}

func dialRunner(t *testing.T, services *usershard.Services, shards *usershard.Shards) *websocket.Conn {
	t.Helper()
	e, err := Start(
		t.Context(),
		netip.MustParseAddrPort("127.0.0.1:0"),
		AppState{Services: services, Shards: shards, Site: &Site{}},
		"",
	)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := e.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	socket, _, err := websocket.Dial(t.Context(), "ws://"+e.LocalAddr().String()+"/api/runner", nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = socket.CloseNow() }) // Cleanup joins the server through Edge.Close.
	return socket
}

func writeRunnerHello(t *testing.T, socket *websocket.Conn, hello *runnerwire.Hello) {
	t.Helper()
	data, err := runnerwire.Encode(hello)
	if err != nil {
		t.Fatal(err)
	}
	if err := socket.Write(t.Context(), websocket.MessageBinary, data); err != nil {
		t.Fatal(err)
	}
}
