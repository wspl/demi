package usershard

import (
	"context"
	"net/netip"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

type heldExposeCreate struct {
	expose.Store
	entered chan struct{}
	resume  chan struct{}
	once    sync.Once
}

func (h *heldExposeCreate) CreateExpose(
	ctx context.Context,
	id webapi.ExposeID,
	user webapi.UserID,
	device webapi.DeviceID,
	address webapi.ExposeAddress,
	lifetime time.Duration,
) (database.ExposeRecord, error) {
	h.once.Do(func() {
		close(h.entered)
	})
	select {
	case <-h.resume:
	case <-ctx.Done():
		return database.ExposeRecord{}, ctx.Err()
	}
	return h.Store.CreateExpose(ctx, id, user, device, address, lifetime)
}

type delayedExposeView struct {
	exposeView
	store expose.Store
}

func (v delayedExposeView) Control() expose.Store { return v.store }

// The Cloud's stop callback races the actual Add commit on a connected device.
// The runner link is inert; no manager or Host access placeholder is executed.
// SQLite and virtual scheduling make the test independent of wall time.
func TestCloudStopOrdersExposeCreation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		clock := core.SystemClock{}
		control := databasetest.Control(t.Context(), t, clock)
		user := databasetest.Master(t.Context(), t, control)
		device, err := control.CreateDevice(
			t.Context(),
			user.ID,
			"Cloud stop fixture",
			runnerwire.RunnerPlatformLinux,
			database.HashToken("fixture"),
		)
		if err != nil {
			t.Fatal(err)
		}
		registry, err := plugins.NewRegistry(nil, func(declare.NativeOperation) bool { return false })
		if err != nil {
			t.Fatal(err)
		}
		domain, err := expose.ParseDomain("expose.localhost")
		if err != nil {
			t.Fatal(err)
		}
		public := &runners.PublicURL{}
		public.Listening(nil, netip.MustParseAddrPort("127.0.0.1:3271"))
		s := &Shard{
			user: user.ID,
			services: &Services{
				Control:      control,
				Clock:        clock,
				Sync:         &pagesync.Registry{},
				ExposeDomain: &domain,
				PublicURL:    public,
			},
		}
		s.plugins = plugins.NewUser(registry, user.ID, s)
		pipes := remotehost.NewPipes(remotehost.Arrival)
		defer func() {
			if err := pipes.Close(context.Background()); err != nil {
				t.Error(err)
			}
		}()
		link, driver := remotehost.NewLink(
			remotehost.LinkOptions{
				Device:   string(device.ID),
				Identity: host.Identity{HomeDir: "/home/test"},
				Pipes:    pipes,
			},
		)
		serving := s.devices.Bind(device.ID, link, driver, runners.NewLastSeen(control, s.Marks()))
		defer serving.Close(context.Background())
		held := &heldExposeCreate{
			Store:   control,
			entered: make(chan struct{}),
			resume:  make(chan struct{}),
		}
		view := delayedExposeView{
			exposeView: exposeView{s},
			store:      exposeStore{Store: held, shard: s},
		}
		added := make(chan error, 1)
		go func() {
			_, err := expose.Add(t.Context(), view, device.ID, "80", time.Hour)
			added <- err
		}()
		<-held.entered
		stopped := make(chan error, 1)
		go func() {
			stopped <- s.CloudStopped(t.Context(), device.ID)
		}()
		synctest.Wait()
		close(held.resume)
		if err := <-added; err != nil {
			t.Fatal(err)
		}
		if err := <-stopped; err != nil {
			t.Fatal(err)
		}
		rows, err := control.UserExposes(t.Context(), user.ID)
		if err != nil {
			t.Fatal(err)
		}
		if len(rows.Live) != 0 {
			t.Fatalf("Cloud stop left %d exposes on its device", len(rows.Live))
		}
	})
}
