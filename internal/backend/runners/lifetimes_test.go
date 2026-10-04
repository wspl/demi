package runners_test

import (
	"context"
	"errors"
	"net/netip"
	"net/url"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
)

func TestConversationFileLeaseBlocksTransition(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		files := runners.NewFileGate("conversation")
		lease, err := files.Enter(t.Context(), gates.Demand)
		if err != nil {
			t.Fatal(err)
		}
		defer lease.Release()
		var devices runners.Devices
		h := devices.ConversationHost("device", lease, "/work space", nil)
		conversation, ok := runners.ConversationOf(h.Key())
		if !ok || conversation != "conversation" {
			t.Fatal("wrong conversation key", h.Key())
		}
		device, ok := runners.DeviceOf(h.Key())
		if !ok || device != "device" {
			t.Fatal("wrong device key", h.Key())
		}
		if h.Online() || devices.DeviceAccess("device") != nil {
			t.Fatal("offline device admitted")
		}
		machine := devices.MachineHost("device", "/home", nil)
		if _, ok := runners.ConversationOf(machine.Key()); ok {
			t.Fatal("machine has conversation identity")
		}
		reserved := make(chan *gates.Reservation, 1)
		go func() {
			reservation, err := files.Gate().Reserve(t.Context())
			if err != nil {
				t.Error(err)
			}
			reserved <- reservation
		}()
		synctest.Wait()
		select {
		case <-reserved:
			t.Fatal("transition passed active file work")
		default:
		}
		lease.Release()
		reservation := <-reserved
		ctx, cancel := context.WithCancel(t.Context())
		attempted := make(chan error, 1)
		go func() {
			next, err := files.Enter(ctx, gates.Maintenance)
			if next != nil {
				next.Release()
			}
			attempted <- err
		}()
		synctest.Wait()
		cancel()
		if err := <-attempted; !errors.Is(err, context.Canceled) {
			t.Fatalf("wait cancellation %v", err)
		}
		reservation.Release()
		if state := files.Gate().State(); state.Demand != 0 || state.Maintenance != 0 || state.Reserved {
			t.Fatalf("lease leak %+v", state)
		}
	})
}

func TestCommandRouterLifetimeAndConversationOwnership(t *testing.T) {
	var router runners.CommandRouter
	commands := &host.CommandSet{}
	catalog := runners.UnpublishedCatalog().Catalog(&runners.PublicURL{})
	selection, err := catalog.Select(commands)
	if err != nil {
		t.Fatal(err)
	}
	first := router.Register("node", "conversation", commands, selection)
	second := router.Register("node", "other", &host.CommandSet{}, nil)
	retained := first.Retain()
	defer first.Release()
	defer second.Release()
	defer retained.Release()
	if router.SelectionOf("node", "conversation") != selection || router.SelectionOf("node", "other") != nil {
		t.Fatal("re-registration replaced owner")
	}
	job := remotehost.JobOrigin{
		Context: cmdproto.Context{Conversation: "conversation"},
		Caller:  &host.JobCaller{Node: "node"},
	}
	assertFailure := func(job remotehost.JobOrigin, kind host.RPCErrorKind, text string) {
		t.Helper()
		_, err := router.Dispatch(t.Context(), job, host.RPCInvocation{Path: []string{"missing"}}, host.RPCPort{})
		var rpc *host.RPCError
		if !errors.As(err, &rpc) || rpc.Kind != kind || rpc.Message != text {
			t.Fatalf("dispatch %v", err)
		}
	}
	assertFailure(remotehost.JobOrigin{}, host.HandlerFailed, "rpc commands run for an agent's jobs")
	wrong := job
	wrong.Context.Conversation = "other"
	assertFailure(wrong, host.HandlerFailed, "node node belongs to another conversation")
	first.Release()
	second.Release()
	// The surviving registration routes to the command set, whose usage check
	// rejects the requested nonexistent command, rather than losing the node.
	assertFailure(job, host.Usage, `"missing" is not an rpc command`)
	retained.Release()
	assertFailure(job, host.HandlerFailed, "no agent session behind node node")
	if router.SelectionOf("node", "conversation") != nil {
		t.Fatal("last release kept the registration")
	}
}

func TestPublicURLPublishesOnceAndUsesLoopback(t *testing.T) {
	for _, address := range []string{"0.0.0.0:3271", "[::]:3271"} {
		var public runners.PublicURL
		if _, ok := public.URL(); ok {
			t.Fatal("listening before bind")
		}
		public.Listening(nil, netip.MustParseAddrPort(address))
		value, ok := public.URL()
		if !ok {
			t.Fatal("listener not published")
		}
		parsed, err := url.Parse(value.String())
		if err != nil {
			t.Fatal(err)
		}
		ip, err := netip.ParseAddr(parsed.Hostname())
		if err != nil || !ip.IsLoopback() {
			t.Fatalf("not loopback %s", value.String())
		}
		public.Listening(nil, netip.MustParseAddrPort("127.0.0.1:99"))
		unchanged, _ := public.URL()
		if unchanged.String() != value.String() {
			t.Fatal("public address replaced")
		}
	}
}
