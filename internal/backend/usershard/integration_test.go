package usershard_test

import (
	"context"
	"errors"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/google/go-cmp/cmp"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/backend/usershard/usershardtest"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

const conversationID webapi.ConversationID = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01"

type fixture struct {
	services *usershard.Services
	shards   *usershard.Shards
	shard    *usershard.Shard
	owner    webapi.UserID
	devices  []database.DeviceRecord
}

func shardFixture(t *testing.T, names ...string) *fixture {
	t.Helper()
	services := usershardtest.StartServices(t)
	owner := databasetest.Master(t.Context(), t, services.Control).ID
	_, created, err := services.Control.CreateConversation(t.Context(), owner, conversationID)
	if err != nil {
		t.Fatal(err)
	}
	if !created {
		t.Fatalf("conversation creation = %T", created)
	}
	shards := usershardtest.StartShards(t, services)
	shard, err := shards.Of(t.Context(), owner)
	if err != nil {
		t.Fatal(err)
	}
	f := &fixture{
		services: services,
		shards:   shards,
		shard:    shard,
		owner:    owner,
	}
	for _, name := range names {
		device, err := services.Control.CreateDevice(
			t.Context(),
			owner,
			name,
			runnerwire.RunnerPlatformLinux,
			database.HashToken(name),
		)
		if err != nil {
			t.Fatal(err)
		}
		f.devices = append(f.devices, device)
	}
	return f
}

func on(device webapi.DeviceID) database.ConversationChange {
	return &database.ConversationTargetChange{
		Target: &webapi.ConversationTargetDevice{DeviceID: device, Path: "/work"},
	}
}

func change(t *testing.T, f *fixture, c database.ConversationChange) {
	t.Helper()
	if err := f.shard.Transition(t.Context(), conversationID, c); err != nil {
		t.Fatal(err)
	}
}

type releaseRecord struct {
	Device   webapi.DeviceID
	Target   webapi.ConversationTarget
	Attached []webapi.DeviceID
	Archived bool
	At       time.Time
}
type releaseLog struct {
	mu      sync.Mutex
	records []releaseRecord
	changed chan struct{}
}

func (l *releaseLog) append(record releaseRecord) {
	l.mu.Lock()
	l.records = append(l.records, record)
	old := l.changed
	l.changed = make(chan struct{})
	l.mu.Unlock()
	close(old)
}

func (l *releaseLog) snapshot() []releaseRecord {
	l.mu.Lock()
	defer l.mu.Unlock()
	return append([]releaseRecord(nil), l.records...)
}

func (l *releaseLog) until(ctx context.Context, count int) error {
	for {
		l.mu.Lock()
		n, wake := len(l.records), l.changed
		l.mu.Unlock()
		if n >= count {
			return nil
		}
		select {
		case <-wake:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

type releaseRunner struct {
	f       *fixture
	device  webapi.DeviceID
	log     *releaseLog
	answers chan []byte
}

func (r *releaseRunner) Receive(ctx context.Context) ([]byte, error) {
	select {
	case answer := <-r.answers:
		return answer, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func (r *releaseRunner) Send(ctx context.Context, data []byte) error {
	message, err := runnerwire.DecodeInbound(data)
	if err != nil {
		return err
	}
	var answer runnerwire.Outbound
	if _, ok := message.(*runnerwire.Ping); ok {
		answer = &runnerwire.Pong{}
	} else if message, ok := message.(*runnerwire.ConversationRelease); ok {
		if err := r.recordRelease(ctx, message); err != nil {
			return err
		}
		answer = &runnerwire.ConversationReleased{ID: message.ID}
	} else {
		return nil
	}
	encoded, err := runnerwire.Encode(answer)
	if err != nil {
		return err
	}
	select {
	case r.answers <- encoded:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func playRunners(t *testing.T, f *fixture) *releaseLog {
	t.Helper()
	log := &releaseLog{changed: make(chan struct{})}
	for _, device := range f.devices {
		link, driver := remotehost.NewLink(
			remotehost.LinkOptions{
				Device: string(device.ID),
				Identity: host.Identity{
					UID:      501,
					GID:      20,
					Hostname: "test",
					HomeDir:  "/home/ana",
				},
				Pipes: f.shard.Pipes(),
			},
		)
		serving := f.shard.Devices().
			Bind(device.ID, link, driver, runners.NewLastSeen(f.services.Control, f.shard.Marks()))
		runner := &releaseRunner{
			f:       f,
			device:  device.ID,
			log:     log,
			answers: make(chan []byte, 8),
		}
		ctx, cancel := context.WithCancel(t.Context())
		done := make(chan struct{})
		go func() {
			defer close(done)
			serving.TestingServe(ctx, runner, runner)
		}()
		t.Cleanup(func() {
			cancel()
			<-done
		})
	}
	return log
}

// All integration scenarios use temporary SQLite storage and in-process runners.
// No test starts a model, machine manager or real process; synctest owns time.
func TestUserAlwaysReachesSameShard(t *testing.T) {
	f := shardFixture(t)
	var workers sync.WaitGroup
	for range 20 {
		workers.Add(1)
		go func() {
			defer workers.Done()
			shard, err := f.shards.Of(t.Context(), f.owner)
			if err != nil {
				t.Error(err)
				return
			}
			if shard.User() != f.owner {
				t.Error("shard returned another user")
			}
			if shard != f.shard {
				t.Error("user reached a second shard")
			}
		}()
	}
	workers.Wait()
}

func TestTransitionRefusesWorkAndFieldUpdateWaits(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := shardFixture(t, "laptop")
		slot := f.shard.Conversations().Slot(conversationID)
		operation, err := slot.FileGate().Enter(t.Context(), gates.Demand)
		if err != nil {
			t.Fatal(err)
		}
		for _, c := range []database.ConversationChange{
			on(f.devices[0].ID),
			&database.ConversationRecordChange{Change: &database.RecordArchived{Archived: true}},
			&database.ConversationRecordChange{Change: &database.RecordDetach{Device: f.devices[0].ID}},
		} {
			err := f.shard.Transition(t.Context(), conversationID, c)
			var refusal *hostaccess.ChangeRefusal
			if !errors.As(err, &refusal) || refusal.Kind != hostaccess.ChangeTurnInFlight {
				t.Errorf("busy transition = %v", err)
			}
		}
		operation.Release()
		held := slot.FileGate().TryReserve()
		if held == nil {
			t.Fatal("gate remained busy")
		}
		done := make(chan error, 1)
		go func() {
			done <- f.shard.Transition(
				t.Context(),
				conversationID,
				&database.ConversationRecordChange{Change: &database.RecordTitle{Title: "Renamed"}},
			)
		}()
		synctest.Wait()
		select {
		case err := <-done:
			t.Fatalf("rename completed while held: %v", err)
		default:
		}
		record, _, err := f.services.Control.Conversation(t.Context(), conversationID)
		if err != nil {
			t.Fatal(err)
		}
		if record.Title == "Renamed" {
			t.Fatal("rename committed during transition")
		}
		held.Release()
		if err := <-done; err != nil {
			t.Fatal(err)
		}
		record, _, err = f.services.Control.Conversation(t.Context(), conversationID)
		if err != nil || record.Title != "Renamed" {
			t.Fatalf("rename = %+v, %v", record, err)
		}
	})
}

func TestSwitchDetachArchiveReleaseBeforeBindingChanges(t *testing.T) {
	f := shardFixture(t, "one", "two")
	log := playRunners(t, f)
	one, two := f.devices[0].ID, f.devices[1].ID
	change(t, f, on(one))
	if len(log.snapshot()) != 0 {
		t.Fatal("unallocated Cloud released a device")
	}
	change(t, f, on(two))
	if got := log.snapshot(); len(got) != 1 {
		t.Fatalf("switch releases = %+v", got)
	}
	change(t, f, &database.ConversationRecordChange{Change: &database.RecordDetach{Device: one}})
	if got := log.snapshot(); len(got) != 2 {
		t.Fatalf("detach releases = %+v", got)
	}
	change(
		t,
		f,
		&database.ConversationRecordChange{
			Change: &database.RecordAttach{
				Host: database.AttachedHostRecord{Device: one, Name: "one"},
			},
		},
	)
	change(t, f, &database.ConversationRecordChange{Change: &database.RecordArchived{Archived: true}})
	got := log.snapshot()
	for i := range got {
		got[i].At = time.Time{}
	}
	target := func(id webapi.DeviceID) webapi.ConversationTarget {
		return &webapi.ConversationTargetDevice{DeviceID: id, Path: "/work"}
	}
	want := []releaseRecord{
		{
			Device:   one,
			Target:   target(one),
			Attached: []webapi.DeviceID{},
		},
		{
			Device:   one,
			Target:   target(two),
			Attached: []webapi.DeviceID{one},
		},
		{
			Device:   two,
			Target:   target(two),
			Attached: []webapi.DeviceID{one},
		},
		{
			Device:   one,
			Target:   target(two),
			Attached: []webapi.DeviceID{one},
		},
	}
	if diff := cmp.Diff(want, got); diff != "" {
		t.Fatal(diff)
	}
}

func operate(t *testing.T, f *fixture) {
	t.Helper()
	_, err := hostaccess.WithHost(
		t.Context(),
		f.shard,
		conversationID,
		nil,
		func(context.Context, *hostaccess.ConversationHost) (struct{}, error) { return struct{}{}, nil },
	)
	if err != nil {
		t.Fatal(err)
	}
}

func TestIdleHourReleasesMainAndAttachedAfterLatestActivity(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := shardFixture(t, "main", "attached")
		log := playRunners(t, f)
		change(t, f, on(f.devices[0].ID))
		change(
			t,
			f,
			&database.ConversationRecordChange{
				Change: &database.RecordAttach{
					Host: database.AttachedHostRecord{Device: f.devices[1].ID, Name: "attached"},
				},
			},
		)
		operate(t, f)
		time.Sleep(30 * time.Minute)
		active := time.Now()
		operate(t, f)
		if err := log.until(t.Context(), 2); err != nil {
			t.Fatal(err)
		}
		got := log.snapshot()
		if len(got) != 2 || got[0].Device != f.devices[0].ID || got[1].Device != f.devices[1].ID {
			t.Fatalf("released = %+v", got)
		}
		for _, release := range got {
			if release.At.Sub(active) < time.Hour {
				t.Fatal("old activity deadline released resources")
			}
		}
		operate(t, f)
	})
}

func TestSwitchRestartsIdleWindowAndArchiveEndsIt(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := shardFixture(t, "old", "new")
		log := playRunners(t, f)
		change(t, f, on(f.devices[0].ID))
		operate(t, f)
		time.Sleep(30 * time.Minute)
		switched := time.Now()
		change(t, f, on(f.devices[1].ID))
		if got := log.snapshot(); len(got) != 1 || got[0].Device != f.devices[0].ID {
			t.Fatalf("switch released = %+v", got)
		}
		if err := log.until(t.Context(), 3); err != nil {
			t.Fatal(err)
		}
		later := log.snapshot()[1:]
		if len(later) != 2 || later[0].Device != f.devices[1].ID || later[1].Device != f.devices[0].ID {
			t.Fatalf("new binding releases = %+v", later)
		}
		for _, release := range later {
			if release.At.Sub(switched) < time.Hour {
				t.Fatal("old binding deadline released the new binding")
			}
		}
		synctest.Wait()
		operate(t, f)
		change(t, f, &database.ConversationRecordChange{Change: &database.RecordArchived{Archived: true}})
		time.Sleep(2 * time.Hour)
		if got := log.snapshot(); len(got) != 5 {
			t.Fatalf("archive releases = %d, want 5", len(got))
		}
	})
}

// recordRelease records the target and attached hosts observed by the test runner.
func (r *releaseRunner) recordRelease(ctx context.Context, message *runnerwire.ConversationRelease) error {
	id, err := webapi.ParseConversationID(message.ConversationID)
	if err != nil {
		return err
	}
	record, _, err := r.f.services.Control.Conversation(ctx, id)
	if err != nil {
		return err
	}
	attached, err := r.f.services.Control.AttachedHosts(ctx, id)
	if err != nil {
		return err
	}
	hosts := make([]webapi.DeviceID, 0, len(attached))
	for _, host := range attached {
		hosts = append(hosts, host.Device)
	}
	r.log.append(
		releaseRecord{
			Device:   r.device,
			Target:   record.Target,
			Attached: hosts,
			Archived: record.Archived,
			At:       time.Now(),
		},
	)
	return nil
}
