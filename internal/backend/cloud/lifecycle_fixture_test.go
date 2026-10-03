package cloud

import (
	"bufio"
	"context"
	"errors"
	"io"
	"net"
	"net/netip"
	"net/url"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/idlewatch"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/runners/runnerstest"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// memoryRecords is a synchronized Cloud storage fixture, not a wire decoder.
// It avoids real filesystem IO in transition timer tests.
type memoryRecords struct {
	mu           sync.Mutex
	device       *database.DeviceRecord
	operations   map[webapi.OperationID]database.ManagedOperation
	latest       *database.ManagedOperation
	uses         []database.CloudUseRecord
	phases       []webapi.ResetPhase
	tokens       []database.TokenHash
	announced    []webapi.OperationID
	writeFailure webapi.ResetPhase
	panicRotate  bool
}

func (r *memoryRecords) ManagedDevice(context.Context, webapi.UserID) (database.DeviceRecord, bool, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.device == nil {
		return database.DeviceRecord{}, false, nil
	}
	v := *r.device
	return v, true, nil
}

func (r *memoryRecords) ManagedDeviceOrCreate(_ context.Context, user webapi.UserID) (database.DeviceRecord, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.device == nil {
		r.device = &database.DeviceRecord{ID: "cloud-device", User: user, Kind: webapi.DeviceKindManaged, Name: "Cloud"}
	}
	return *r.device, nil
}

func (r *memoryRecords) LatestManagedOperation(
	context.Context,
	webapi.DeviceID,
) (database.ManagedOperation, bool, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.latest == nil {
		return database.ManagedOperation{}, false, nil
	}
	v := *r.latest
	return v, true, nil
}

func (r *memoryRecords) ManagedOperation(
	_ context.Context,
	_ webapi.DeviceID,
	id webapi.OperationID,
) (database.ManagedOperation, bool, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	v, ok := r.operations[id]
	if !ok {
		return database.ManagedOperation{}, false, nil
	}
	return v, true, nil
}

func (r *memoryRecords) PutManagedOperation(_ context.Context, _ webapi.DeviceID, op database.ManagedOperation) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if op.Phase == r.writeFailure {
		return io.ErrUnexpectedEOF
	}
	r.operations[op.ID] = op
	r.latest = &op
	r.phases = append(r.phases, op.Phase)
	return nil
}

func (r *memoryRecords) RotateDeviceToken(_ context.Context, _ webapi.DeviceID, token database.TokenHash) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.panicRotate {
		panic("scripted storage task failure")
	}
	r.tokens = append(r.tokens, token)
	return nil
}

func (r *memoryRecords) AnnounceCloudReset(_ context.Context, _ webapi.UserID, id webapi.OperationID) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.announced = append(r.announced, id)
	return nil
}

func (r *memoryRecords) CloudUses(context.Context, webapi.UserID, *webapi.DeviceID) ([]database.CloudUseRecord, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]database.CloudUseRecord(nil), r.uses...), nil
}

func (r *memoryRecords) Device(ctx context.Context, id webapi.DeviceID) (database.DeviceRecord, bool, error) {
	v, found, err := r.ManagedDevice(ctx, "")
	if found && v.ID != id {
		return database.DeviceRecord{}, false, nil
	}
	return v, found, err
}

type cloudFixture struct {
	t        *testing.T
	cloud    *Cloud
	services *Services
	devices  runners.Devices
	public   runners.PublicURL
	marks    pagesync.UserMarks
	records  *memoryRecords
	// mu protects scripted observations; scripts change only when workers are quiescent.
	mu                sync.Mutex
	calls             []machinewire.MachineCall
	changed           chan struct{}
	hook              func(machinewire.MachineCall) (string, error)
	runtime           machinewire.RuntimeState
	connect           bool
	flush             bool
	activity          map[webapi.ConversationID]idlewatch.Activity
	attended          map[webapi.ConversationID]bool
	holds             int
	heldIDs           []webapi.ConversationID
	resetFiles        []bool
	holdFailure       webapi.ConversationID
	stops             int
	panicStop         bool
	pipes             *remotehost.Pipes
	control           *database.ControlService
	runnerConnections []*runnerstest.Connection
	ctx               context.Context
	cancel            context.CancelFunc
	workers           sync.WaitGroup
}

func (f *cloudFixture) User() webapi.UserID               { return "owner" }
func (f *cloudFixture) Cloud() *Cloud                     { return f.cloud }
func (f *cloudFixture) CloudServices() *Services          { return f.services }
func (f *cloudFixture) Devices() *runners.Devices         { return &f.devices }
func (f *cloudFixture) Control() *database.ControlService { return f.control }
func (f *cloudFixture) PublicURL() *runners.PublicURL     { return &f.public }
func (f *cloudFixture) IdleWindow() time.Duration         { return 10 * time.Second }
func (f *cloudFixture) Marks() pagesync.UserMarks         { return f.marks }
func (f *cloudFixture) Vault() *providers.Vault           { return nil }
func (f *cloudFixture) Assembly() *providers.Assembly     { return nil }
func (f *cloudFixture) Activity(id webapi.ConversationID) idlewatch.Activity {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.activity[id]
}

func (f *cloudFixture) Attended(id webapi.ConversationID) bool {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.attended[id]
}

func (f *cloudFixture) HoldForIdle(id webapi.ConversationID) ConversationHold {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.activity[id].Busy || f.holdFailure == id {
		return nil
	}
	f.holds++
	f.heldIDs = append(f.heldIDs, id)
	return &fixtureHold{f: f}
}

func (f *cloudFixture) HoldForReset(
	_ context.Context,
	id webapi.ConversationID,
	files bool,
	_ time.Duration,
) (ConversationHold, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.holdFailure == id {
		return nil, ErrNotLetGo
	}
	f.holds++
	f.heldIDs = append(f.heldIDs, id)
	f.resetFiles = append(f.resetFiles, files)
	return &fixtureHold{f: f}, nil
}

func (f *cloudFixture) CloudStopped(context.Context, webapi.DeviceID) error {
	if f.panicStop {
		panic("scripted stop task failure")
	}
	f.mu.Lock()
	f.stops++
	f.mu.Unlock()
	return nil
}

type fixtureHold struct {
	f    *cloudFixture
	once sync.Once
}

func (h *fixtureHold) Release() {
	h.once.Do(func() {
		h.f.mu.Lock()
		h.f.holds--
		h.f.mu.Unlock()
	})
}

// newCloudFixture runs the real client and Cloud lifecycle over a scripted
// manager. Cloud storage uses memoryRecords; runner teardown alone uses the
// real last-seen database, outside any lifecycle timing assertion.
func newCloudFixture(t *testing.T) *cloudFixture {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	f := &cloudFixture{
		t:        t,
		ctx:      ctx,
		cancel:   cancel,
		changed:  make(chan struct{}),
		connect:  true,
		runtime:  machinewire.RuntimeStateRunning,
		activity: make(map[webapi.ConversationID]idlewatch.Activity),
		attended: make(map[webapi.ConversationID]bool),
	}
	f.records = &memoryRecords{operations: make(map[webapi.OperationID]database.ManagedOperation)}
	f.cloud = New(ctx, &sync.Mutex{})
	f.cloud.records = f.records
	f.marks = (&pagesync.SyncRegistry{}).Of(f.User())
	f.pipes = remotehost.NewPipes(remotehost.Arrival)
	f.control = databasetest.Control(ctx, t, nil)
	// Last-seen persistence is deliberately unavailable in this fixture. Cloud
	// storage is memoryRecords; a runner must still disconnect when ancillary
	// last-seen storage is closed, as runnerstest itself tests.
	if err := f.control.Close(ctx); err != nil {
		t.Fatal(err)
	}
	tuning := DefaultTuning()
	tuning.Sweep = time.Second
	tuning.CheckpointInterval = 100 * time.Second
	tuning.LifetimeCap = 1000 * time.Second
	tuning.RunnerConnection = 3 * time.Second
	tuning.SyncTimeout = time.Second
	tuning.ResetHold = 2 * time.Second
	c, _ := NewClient(ctx, "scripted-cloud")
	c.dial = func(context.Context, string, string) (net.Conn, error) {
		local, peer := net.Pipe()
		f.workers.Go(func() { f.serveManager(peer) })
		return local, nil
	}
	f.services = NewServices(c, tuning)
	backend, err := url.Parse("http://backend.test")
	if err != nil {
		t.Fatal(err)
	}
	f.public.Listening(backend, netip.AddrPort{})
	t.Cleanup(func() {
		if err := Close(context.Background(), f); err != nil {
			t.Error(err)
		}
		if err := c.Close(context.Background()); err != nil {
			t.Error(err)
		}
		cancel()
		f.workers.Wait()
		for _, connection := range f.runnerConnections {
			if err := connection.Close(context.Background()); err != nil {
				t.Error(err)
			}
		}
		if err := f.pipes.Close(context.Background()); err != nil {
			t.Error(err)
		}
		f.mu.Lock()
		defer f.mu.Unlock()
		if f.holds != 0 {
			t.Errorf("conversation holds leaked: %d", f.holds)
		}
		f.cloud.mu.Lock()
		m := f.cloud.machine
		f.cloud.mu.Unlock()
		if m != nil {
			state := m.gate.State()
			if state.Demand != 0 || state.Maintenance != 0 || state.Reserved {
				t.Errorf("gate leaked: %+v", state)
			}
		}
	})
	return f
}

// connectRunner publishes a real runner link and gives Serving ownership to runnerstest.
func (f *cloudFixture) connectRunner() {
	link, driver := remotehost.NewLink(
		remotehost.LinkOptions{
			Device:   "cloud-device",
			Identity: host.Identity{HomeDir: "/home/demi"},
			Pipes:    f.pipes,
			Policy:   remotehosttest.NewCommandPolicy(nil),
		},
	)
	serving := f.devices.Bind("cloud-device", link, driver, runners.NewLastSeen(f.control, f.marks))
	incoming, outgoing := make(cloudFrames, 64), make(cloudFrames, 64)
	connection := runnerstest.Serve(f.t, serving, incoming, outgoing)
	f.mu.Lock()
	f.runnerConnections = append(f.runnerConnections, connection)
	f.mu.Unlock()
	f.workers.Go(func() {
		for {
			select {
			case <-f.ctx.Done():
				return
			case frame := <-outgoing:
				message, err := runnerwire.DecodeInbound(frame)
				if err != nil {
					f.t.Error(err)
					return
				}
				if request, ok := message.(*runnerwire.Sync); ok && f.flush {
					data, err := runnerwire.Encode(&runnerwire.SyncDone{ID: request.ID})
					if err != nil {
						f.t.Error(err)
						return
					}
					select {
					case incoming <- data:
					case <-f.ctx.Done():
						return
					}
				}
			}
		}
	})
}

type cloudFrames chan []byte

func (f cloudFrames) Receive(ctx context.Context) ([]byte, error) {
	select {
	case data := <-f:
		return data, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func (f cloudFrames) Send(ctx context.Context, data []byte) error {
	select {
	case f <- data:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// serveManager scripts requests through the actual manager codecs, allowing
// concurrent replies so a blocked checkpoint cannot serialize unrelated calls.
func (f *cloudFixture) serveManager(conn net.Conn) {
	defer func() { _ = conn.Close() }() // Closing the scripted connection only releases its descriptor.
	ctx, cancel := context.WithCancel(f.ctx)
	defer cancel()
	closed := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		// Cancellation owns descriptor cleanup; the peer may already be gone.
		_ = conn.Close()
		close(closed)
	})
	defer func() {
		if !stop() {
			<-closed
		}
	}()
	responses := make(chan machinewire.MachineResponse, 64)
	var replies sync.WaitGroup
	writerDone := make(chan struct{})
	go func() {
		defer close(writerDone)
		for {
			select {
			case <-ctx.Done():
				return
			case response := <-responses:
				data, err := machinewire.EncodeLine(response)
				if err != nil {
					f.t.Error(err)
					return
				}
				if _, err := conn.Write(data); err != nil {
					return
				}
			}
		}
	}()
	defer func() {
		cancel()
		<-writerDone
		replies.Wait()
	}()
	scanner := bufio.NewScanner(conn)
	for scanner.Scan() {
		request, err := machinewire.DecodeRequest(scanner.Bytes())
		if err != nil {
			f.t.Error(err)
			return
		}
		f.mu.Lock()
		f.calls = append(f.calls, request.Call)
		changed := f.changed
		f.changed = make(chan struct{})
		f.mu.Unlock()
		close(changed)
		replies.Go(func() {
			result, err := f.answer(request.Call)
			var response machinewire.MachineResponse = &machinewire.OK{ID: request.ID, Result: []byte(result)}
			if err != nil {
				response = &machinewire.ErrorResponse{ID: request.ID, Message: err.Error()}
			}
			select {
			case responses <- response:
			case <-ctx.Done():
			}
		})
	}
}

func (f *cloudFixture) answer(call machinewire.MachineCall) (string, error) {
	if f.hook != nil {
		result, err := f.hook(call)
		if result != "" || err != nil {
			return result, err
		}
	}
	switch call.Name() {
	case "current_base_version":
		return `"base-1"`, nil
	case "runtime_state":
		return `"` + string(f.runtime) + `"`, nil
	case "image_state":
		return `{"generation":"gen-1","baseVersion":"base-1","resetId":null,"systemBytes":1024,"homeBytes":2048}`, nil
	case "wake":
		if f.connect {
			f.connectRunner()
		}
	}
	return "null", nil
}

func (f *cloudFixture) count(name string) int {
	f.mu.Lock()
	defer f.mu.Unlock()
	n := 0
	for _, call := range f.calls {
		if call.Name() == name {
			n++
		}
	}
	return n
}

func (f *cloudFixture) waitCalls(ctx context.Context, name string, count int) {
	f.t.Helper()
	for {
		f.mu.Lock()
		changed := f.changed
		f.mu.Unlock()
		if f.count(name) >= count {
			return
		}
		select {
		case <-changed:
		case <-ctx.Done():
			f.t.Fatal(ctx.Err())
		}
	}
}

func (f *cloudFixture) access() *MachineAccess {
	f.t.Helper()
	a, err := Access(f.t.Context(), f)
	if err != nil {
		f.t.Fatal(err)
	}
	return a
}

func (f *cloudFixture) phase() webapi.CloudState {
	f.cloud.mu.Lock()
	defer f.cloud.mu.Unlock()
	if f.cloud.machine == nil {
		return webapi.CloudStateUnallocated
	}
	return f.cloud.machine.phase
}

func requireCloudError(t *testing.T, err error, kind ErrorKind) {
	t.Helper()
	var cloud *Error
	if !errors.As(err, &cloud) || cloud.Kind != kind {
		t.Fatalf("want Cloud kind %v, got %v", kind, err)
	}
}
