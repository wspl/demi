package expose_test

import (
	"context"
	"errors"
	"os"
	"sort"
	"sync"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

const testID webapiproto.ExposeID = "k7x2maqw4p3s6tavaw2y4z6aab"

// These in-process scenarios cost no network or wall-clock waits. Storage is
// controlled to interleave operations without external resources.
type shard struct {
	store     *memoryStore
	exposes   expose.Connections
	domain    *expose.Domain
	backend   *url.Url
	connected bool
	changes   atomic.Int32
}

func (s *shard) User() webapiproto.UserID {
	return "owner"
}

func (s *shard) Control() expose.Store {
	return s.store
}

func (s *shard) Clock() types.Clock {
	return types.SystemClock{}
}

func (s *shard) ExposesChanged() {
	s.changes.Add(1)
}

func (s *shard) Exposes() *expose.Connections {
	return &s.exposes
}

func (s *shard) Domain() *expose.Domain {
	return s.domain
}

func (s *shard) PublicURL() *url.Url {
	return s.backend
}

func (s *shard) DeviceConnected(database.DeviceRecord) bool {
	return s.connected
}

func newShard(t *testing.T) *shard {
	t.Helper()
	domain, err := expose.ParseDomain("expose.localhost")
	if err != nil {
		t.Fatal(err)
	}
	backend, err := url.Parse("http://localhost:3271")
	if err != nil {
		t.Fatal(err)
	}
	s := &shard{
		domain:    &domain,
		backend:   backend,
		connected: true,
		store:     &memoryStore{records: make(map[webapiproto.ExposeID]database.ExposeRecord)},
	}
	t.Cleanup(func() {
		if err := s.exposes.Close(context.Background()); err != nil {
			t.Error(err)
		}
		if n := s.exposes.Active(); n != 0 {
			t.Errorf("leaked %d admissions", n)
		}
	})
	return s
}

type memoryStore struct {
	mu      sync.Mutex
	records map[webapiproto.ExposeID]database.ExposeRecord
	read    func(context.Context) error
	failure error
}

func (m *memoryStore) Device(_ context.Context, id webapiproto.DeviceID) (database.DeviceRecord, bool, error) {
	if id == "missing" {
		return database.DeviceRecord{}, false, nil
	}
	user := webapiproto.UserID("owner")
	if id == "foreign" {
		user = "other"
	}
	return database.DeviceRecord{ID: id, User: user}, m.failure == nil, m.failure
}

func (m *memoryStore) CreateExpose(
	_ context.Context,
	id webapiproto.ExposeID,
	user webapiproto.UserID,
	device webapiproto.DeviceID,
	address webapiproto.ExposeAddress,
	lifetime time.Duration,
) (database.ExposeRecord, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.failure != nil {
		return database.ExposeRecord{}, m.failure
	}
	expiry, err := types.TimestampFromTime(time.Now().Add(lifetime))
	if err != nil {
		return database.ExposeRecord{}, err
	}
	record := database.ExposeRecord{
		ID:        id,
		User:      user,
		Device:    device,
		Address:   address,
		CreatedAt: types.SystemClock{}.Now(),
		ExpiresAt: expiry,
	}
	m.records[id] = record
	return record, nil
}

func (m *memoryStore) Expose(ctx context.Context, id webapiproto.ExposeID) (database.ExposeRecord, bool, error) {
	if m.read != nil {
		if err := m.read(ctx); err != nil {
			return database.ExposeRecord{}, false, err
		}
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.failure != nil {
		return database.ExposeRecord{}, false, m.failure
	}
	record, ok := m.records[id]
	if !ok {
		return database.ExposeRecord{}, false, nil
	}
	return record, true, nil
}

func (m *memoryStore) UserExposes(_ context.Context, user webapiproto.UserID) (database.UserExposes, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.failure != nil {
		return database.UserExposes{}, m.failure
	}
	result := database.UserExposes{}
	for id, record := range m.records {
		if record.User != user {
			continue
		}
		if record.ExpiresAt <= (types.SystemClock{}).Now() {
			delete(m.records, id)
			result.Expired = append(result.Expired, id)
		} else {
			result.Live = append(result.Live, record)
		}
	}
	sort.Slice(result.Live, func(i, j int) bool {
		return result.Live[i].ExpiresAt < result.Live[j].ExpiresAt
	})
	return result, nil
}

func (m *memoryStore) RenewExpose(
	_ context.Context,
	id webapiproto.ExposeID,
	user webapiproto.UserID,
	lifetime time.Duration,
) (database.ExposeRecord, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.failure != nil {
		return database.ExposeRecord{}, m.failure
	}
	record, ok := m.records[id]
	if !ok || record.User != user || record.ExpiresAt <= (types.SystemClock{}).Now() {
		return database.ExposeRecord{}, database.ErrExposeNotFound
	}
	expiry, err := types.TimestampFromTime(time.Now().Add(lifetime))
	if err != nil {
		return database.ExposeRecord{}, err
	}
	record.ExpiresAt = expiry
	m.records[id] = record
	return record, nil
}

func (m *memoryStore) DeleteExpose(_ context.Context, id webapiproto.ExposeID) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.failure != nil {
		return m.failure
	}
	delete(m.records, id)
	return nil
}

func (m *memoryStore) DeleteExpiredExpose(
	_ context.Context,
	id webapiproto.ExposeID,
	at types.Timestamp,
) (bool, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.failure != nil {
		return false, m.failure
	}
	record, ok := m.records[id]
	if !ok || record.ExpiresAt > at {
		return false, nil
	}
	delete(m.records, id)
	return true, nil
}

func (m *memoryStore) DeleteDeviceExposes(
	_ context.Context,
	device webapiproto.DeviceID,
) ([]webapiproto.ExposeID, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.failure != nil {
		return nil, m.failure
	}
	var ids []webapiproto.ExposeID
	for id, record := range m.records {
		if record.Device == device {
			ids = append(ids, id)
			delete(m.records, id)
		}
	}
	return ids, nil
}

func add(t *testing.T, s *shard, duration time.Duration) expose.Expose {
	t.Helper()
	value, err := expose.Add(t.Context(), s, "device", "127.0.0.1:5173", duration)
	if err != nil {
		t.Fatal(err)
	}
	return value
}

func admit(t *testing.T, s *shard, id webapiproto.ExposeID) *expose.RelayAdmission {
	t.Helper()
	admission, err := expose.AdmitRelay(t.Context(), s, id)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(admission.Release)
	return admission
}

func TestExposeURLTakesBackendSchemeAndNondefaultPort(t *testing.T) {
	for _, tc := range []struct{ backend, domain, want string }{
		{"http://localhost:3271", "expose.localhost", "http://" + string(testID) + ".expose.localhost:3271/"},
		{"https://demi.example", "expose.demi.example", "https://" + string(testID) + ".expose.demi.example/"},
		{"https://demi.example:443/", "expose.demi.example", "https://" + string(testID) + ".expose.demi.example/"},
		{"https://demi.example/api", "expose.demi.example", "https://" + string(testID) + ".expose.demi.example/"},
	} {
		t.Run(tc.backend, func(t *testing.T) {
			backend, err := url.Parse(tc.backend)
			if err != nil {
				t.Fatal(err)
			}
			domain, err := expose.ParseDomain(tc.domain)
			if err != nil {
				t.Fatal(err)
			}
			if got := expose.URL(testID, domain, backend); got != tc.want {
				t.Fatalf("URL = %s, want %s", got, tc.want)
			}
		})
	}
}

func TestDomainRouting(t *testing.T) {
	for _, tc := range []struct{ input, want string }{
		{"EXPOSE.Example", "expose.example"},
		{"bücher.example", "xn--bcher-kva.example"},
		{"expose%2eexample", "expose.example"},
		{"", ""},
		{"127.0.0.1", ""},
		{"0x7f000001", ""},
		{"[::1]", ""},
		{"expose..example", ""},
		{"example.", ""},
		{" example", ""},
		{"example:80", ""},
		{"example/path", ""},
		{"example%2fpath", ""},
		{"user@example", ""},
	} {
		t.Run(tc.input, func(t *testing.T) {
			domain, err := expose.ParseDomain(tc.input)
			if tc.want == "" {
				if !errors.Is(err, expose.ErrNotDomain) {
					t.Fatalf("error = %v", err)
				}
				return
			}
			if err != nil || domain.String() != tc.want {
				t.Fatalf("domain = %s, %v", domain, err)
			}
		})
	}
	domain, err := expose.ParseDomain("expose.example")
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		host, label string
		ok          bool
	}{
		{"ABC.EXPOSE.EXAMPLE", "abc", true},
		{"expose.example", "", false},
		{".expose.example", "", false},
		{"a.b.expose.example", "", false},
		{"a.expose.example:80", "", false},
		{"a.other.example", "", false},
	} {
		label, ok := domain.Label(tc.host)
		if ok != tc.ok || (ok && label != tc.label) {
			t.Errorf("Label(%q) = %q, %v", tc.host, label, ok)
		}
	}
}

func TestRecordLifecycleAndOwnership(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newShard(t)
		first := add(t, s, time.Hour)
		second := add(t, s, 2*time.Hour)
		if first.Record.ID == second.Record.ID {
			t.Fatal("duplicate credentials")
		}
		if _, err := webapiproto.ParseExposeID(string(first.Record.ID)); err != nil {
			t.Fatal(err)
		}
		listed, err := expose.List(t.Context(), s)
		if err != nil || len(listed) != 2 || listed[0].Record.ID != first.Record.ID {
			t.Fatalf("list = %v, %v", listed, err)
		}
		admission := admit(t, s, first.Record.ID)
		renewed, err := expose.Renew(t.Context(), s, first.Record.ID, 3*time.Hour)
		if err != nil || renewed.Record.ExpiresAt <= second.Record.ExpiresAt {
			t.Fatalf("renew = %v, %v", renewed, err)
		}
		if err := expose.Remove(t.Context(), s, first.Record.ID); err != nil {
			t.Fatal(err)
		}
		<-admission.Ending()
		if err := expose.Remove(t.Context(), s, first.Record.ID); !errors.Is(err, expose.ErrNotFound) {
			t.Fatalf("remove again = %v", err)
		}
		s.store.mu.Lock()
		record := s.store.records[second.Record.ID]
		record.User = "other"
		s.store.records[record.ID] = record
		s.store.mu.Unlock()
		if _, err := expose.Renew(t.Context(), s, record.ID, time.Hour); !errors.Is(err, expose.ErrNotFound) {
			t.Fatalf("foreign renewal = %v", err)
		}
		if err := expose.Remove(t.Context(), s, record.ID); !errors.Is(err, expose.ErrNotFound) {
			t.Fatalf("foreign remove = %v", err)
		}
		if _, err := expose.AdmitRelay(t.Context(), s, record.ID); !errors.Is(err, expose.ErrRelayNotFound) {
			t.Fatalf("foreign relay = %v", err)
		}
		listed, err = expose.List(t.Context(), s)
		if err != nil || len(listed) != 0 {
			t.Fatalf("foreign list = %v, %v", listed, err)
		}
		if s.changes.Load() != 4 {
			t.Fatalf("change notifications = %d", s.changes.Load())
		}
	})
}

func TestCreationRefusalsAndDisabledInstance(t *testing.T) {
	s := newShard(t)
	for _, device := range []webapiproto.DeviceID{"missing", "foreign"} {
		if _, err := expose.Add(t.Context(), s, device, "80", time.Hour); !errors.Is(err, expose.ErrDeviceNotFound) {
			t.Fatalf("device %s: %v", device, err)
		}
	}
	s.connected = false
	if _, err := expose.Add(t.Context(), s, "device", "80", time.Hour); !errors.Is(err, expose.ErrDeviceOffline) {
		t.Fatalf("offline: %v", err)
	}
	s.domain = nil
	if _, err := expose.Add(t.Context(), s, "device", "80", time.Hour); !errors.Is(err, expose.ErrUnavailable) {
		t.Fatalf("disabled: %v", err)
	}
	list, err := expose.List(t.Context(), s)
	if err != nil || len(list) != 0 {
		t.Fatalf("disabled list: %v, %v", list, err)
	}
	if _, err := expose.Renew(t.Context(), s, testID, time.Hour); !errors.Is(err, expose.ErrNotFound) {
		t.Fatal(err)
	}
	if err := expose.Remove(t.Context(), s, testID); !errors.Is(err, expose.ErrNotFound) {
		t.Fatal(err)
	}
}

func TestAdmissionLimitAndRelease(t *testing.T) {
	s := newShard(t)
	value := add(t, s, time.Hour)
	var admissions []*expose.RelayAdmission
	var mu sync.Mutex
	var wg sync.WaitGroup
	for range 64 {
		wg.Go(func() {
			a, err := expose.AdmitRelay(t.Context(), s, value.Record.ID)
			if err != nil {
				t.Error(err)
				return
			}
			mu.Lock()
			admissions = append(admissions, a)
			mu.Unlock()
		})
	}
	wg.Wait()
	defer func() {
		for _, a := range admissions {
			a.Release()
		}
	}()
	if _, err := expose.AdmitRelay(t.Context(), s, value.Record.ID); !errors.Is(err, expose.ErrLimit) {
		t.Fatalf("65th: %v", err)
	}
	admissions[0].Release()
	admissions[0].Release()
	a := admit(t, s, value.Record.ID)
	if a.Record().Address != value.Record.Address {
		t.Fatal("wrong destination")
	}
	s.exposes.EndAll()
	<-a.Ending()
	if s.exposes.Active() != 64 {
		t.Fatal("revocation lost outstanding leases")
	}
}

func TestRemovalDuringAdmission(t *testing.T) {
	s := newShard(t)
	value := add(t, s, time.Hour)
	entered := make(chan struct{})
	resume := make(chan struct{})
	s.store.read = func(ctx context.Context) error {
		close(entered)
		select {
		case <-resume:
			return nil
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	result := make(chan error, 1)
	go func() {
		a, err := expose.AdmitRelay(t.Context(), s, value.Record.ID)
		if a != nil {
			a.Release()
		}
		result <- err
	}()
	<-entered
	s.exposes.End([]webapiproto.ExposeID{value.Record.ID})
	close(resume)
	if err := <-result; !errors.Is(err, expose.ErrRemoved) {
		t.Fatalf("admission after destruction: %v", err)
	}
}

func TestExpiryFollowsRenewalAndEndsRelay(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newShard(t)
		start := time.Now()
		value := add(t, s, time.Hour)
		a := admit(t, s, value.Record.ID)
		done := make(chan struct{})
		ended := false
		go func() {
			defer close(done)
			a.Relay(t.Context(), s, nil, func() {
				ended = true
			})
		}()
		synctest.Wait()
		time.Sleep(30 * time.Minute)
		if _, err := expose.Renew(t.Context(), s, value.Record.ID, time.Hour); err != nil {
			t.Fatal(err)
		}
		<-done
		if !ended || time.Since(start) != 90*time.Minute {
			t.Fatalf("relay ended=%v at %v", ended, time.Since(start))
		}
		record, found, err := s.store.Expose(t.Context(), value.Record.ID)
		if err != nil || found {
			t.Fatalf("expired record: %v, %v", record, err)
		}
		if s.exposes.Active() != 0 {
			t.Fatal("expiry leaked lease")
		}
	})
}

func TestLastReleaseStopsExpiryWatch(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newShard(t)
		value := add(t, s, time.Hour)
		a := admit(t, s, value.Record.ID)
		released := make(chan struct{})
		done := make(chan struct{})
		go func() {
			defer close(done)
			a.Relay(t.Context(), s, released, func() {})
		}()
		synctest.Wait()
		close(released)
		<-done
		synctest.Wait()
		time.Sleep(2 * time.Hour)
		record, found, err := s.store.Expose(t.Context(), value.Record.ID)
		if err != nil || !found {
			t.Fatalf("unobserved record should expire on next read: %v, %v", record, err)
		}
		if _, err := expose.AdmitRelay(t.Context(), s, value.Record.ID); !errors.Is(err, expose.ErrRelayNotFound) {
			t.Fatal(err)
		}
	})
}

func TestExpiredReadsAndDeviceDestruction(t *testing.T) {
	for _, operation := range []string{"list", "renew", "remove", "device"} {
		t.Run(operation, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				s := newShard(t)
				value := add(t, s, time.Hour)
				a := admit(t, s, value.Record.ID)
				time.Sleep(time.Hour)
				switch operation {
				case "list":
					list, err := expose.List(t.Context(), s)
					if err != nil || len(list) != 0 {
						t.Fatalf("list: %v, %v", list, err)
					}
				case "renew":
					_, err := expose.Renew(t.Context(), s, value.Record.ID, time.Hour)
					if !errors.Is(err, expose.ErrNotFound) {
						t.Fatal(err)
					}
				case "remove":
					if err := expose.Remove(t.Context(), s, value.Record.ID); !errors.Is(err, expose.ErrNotFound) {
						t.Fatal(err)
					}
				case "device":
					expose.DestroyOn(t.Context(), s, value.Record.Device)
				}
				<-a.Ending()
				record, found, err := s.store.Expose(t.Context(), value.Record.ID)
				if err != nil || found {
					t.Fatalf("record: %v, %v", record, err)
				}
			})
		})
	}
}

func TestStorageFailureReleasesAdmission(t *testing.T) {
	s := newShard(t)
	s.store.failure = os.ErrPermission
	if _, err := expose.AdmitRelay(t.Context(), s, testID); !errors.Is(err, os.ErrPermission) {
		t.Fatal(err)
	}
	if s.exposes.Active() != 0 {
		t.Fatal("failed read leaked admission")
	}
	if _, err := expose.Add(t.Context(), s, "device", "80", time.Hour); !errors.Is(err, os.ErrPermission) {
		t.Fatal(err)
	}
}

func TestCloseCancelsRelaysAndRejectsAdmission(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newShard(t)
		value := add(t, s, time.Hour)
		a := admit(t, s, value.Record.ID)
		done := make(chan struct{})
		go func() {
			defer close(done)
			a.Relay(t.Context(), s, nil, func() {})
		}()
		synctest.Wait()
		if err := s.exposes.Close(t.Context()); err != nil {
			t.Fatal(err)
		}
		<-done
		if _, err := expose.AdmitRelay(t.Context(), s, value.Record.ID); !errors.Is(err, expose.ErrRemoved) {
			t.Fatal(err)
		}
	})
}

func TestSharedExpirySurvivesOneConnectionClosing(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newShard(t)
		value := add(t, s, time.Hour)
		first := admit(t, s, value.Record.ID)
		second := admit(t, s, value.Record.ID)
		released := make(chan struct{})
		firstDone := make(chan struct{})
		secondDone := make(chan struct{})
		go func() {
			defer close(firstDone)
			first.Relay(t.Context(), s, released, func() {})
		}()
		go func() {
			defer close(secondDone)
			second.Relay(t.Context(), s, nil, func() {})
		}()
		synctest.Wait()
		start := time.Now()
		close(released)
		<-firstDone
		<-secondDone
		if time.Since(start) != time.Hour {
			t.Fatalf("remaining connection ended at %v", time.Since(start))
		}
	})
}

func TestCanceledRelayReleasesAdmission(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newShard(t)
		value := add(t, s, time.Hour)
		admission := admit(t, s, value.Record.ID)
		ctx, cancel := context.WithCancel(t.Context())
		done := make(chan struct{})
		ended := 0
		go func() {
			defer close(done)
			admission.Relay(ctx, s, nil, func() {
				ended++
			})
		}()
		synctest.Wait()
		cancel()
		<-done
		if ended != 1 || s.exposes.Active() != 0 {
			t.Fatalf("end calls=%d, active=%d", ended, s.exposes.Active())
		}
	})
}

func TestExpiryStorageFailureDoesNotRetryOrEndConnection(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newShard(t)
		value := add(t, s, time.Hour)
		admission := admit(t, s, value.Record.ID)
		released := make(chan struct{})
		done := make(chan struct{})
		var reads atomic.Int32
		s.store.read = func(context.Context) error {
			reads.Add(1)
			return os.ErrPermission
		}
		go func() {
			defer close(done)
			admission.Relay(t.Context(), s, released, func() {})
		}()
		synctest.Wait()
		time.Sleep(2 * time.Hour)
		synctest.Wait()
		if reads.Load() != 1 {
			t.Fatalf("expiry read attempts=%d", reads.Load())
		}
		select {
		case <-admission.Ending():
			t.Fatal("storage failure ended connection")
		default:
		}
		close(released)
		<-done
	})
}
