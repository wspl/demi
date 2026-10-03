package runners

import (
	"context"
	"io"
	"log/slog"
	"sync"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// Devices holds the user's devices and their connection slots. Its zero value
// is ready to use. Methods synchronize slot access; the shard serializes adoption
// decisions (Settled, duplicate refusal, and Bind). Do not copy it.
type Devices struct {
	mu    sync.Mutex // Protects the slot registry; individual slots publish snapshots.
	slots map[webapi.DeviceID]*deviceSlot
}
type deviceSlot struct {
	mu      sync.Mutex // Protects connection snapshots and change-channel replacement.
	current remotehost.DeviceLink
	changed chan struct{}
}

// Link returns the device's live connection, or nil while offline.
func (d *Devices) Link(device webapi.DeviceID) *remotehost.Link {
	slot := d.slot(device, false)
	if slot == nil {
		return nil
	}
	current, _ := slot.snapshot()
	if current.Link != nil && !current.Link.IsClosed() {
		return current.Link
	}
	return nil
}

// Online reports whether the device has a live runner connection.
func (d *Devices) Online(device webapi.DeviceID) bool { return d.Link(device) != nil }

// UntilOnline waits until a live connection serves device, as a Cloud boot does.
func (d *Devices) UntilOnline(ctx context.Context, device webapi.DeviceID) error {
	slot := d.slot(device, true)
	for {
		current, changed := slot.snapshot()
		if current.Link != nil && !current.Link.IsClosed() {
			return nil
		}
		select {
		case <-changed:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

// Settled waits for a closing connection to finish and mark its slot offline.
func (d *Devices) Settled(ctx context.Context, device webapi.DeviceID) error {
	slot := d.slot(device, true)
	for {
		current, changed := slot.snapshot()
		if current.Link == nil || !current.Link.IsClosed() {
			return nil
		}
		select {
		case <-changed:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

// Home returns the runner's last reported home, or false before its first connection.
func (d *Devices) Home(device webapi.DeviceID) (string, bool) {
	slot := d.slot(device, false)
	if slot == nil {
		return "", false
	}
	current, _ := slot.snapshot()
	if current.Link != nil {
		return current.Link.Identity().HomeDir, true
	}
	if current.Last != nil {
		return current.Last.HomeDir, true
	}
	return "", false
}

// ConversationHost makes the conversation's Host for the operation holding files.
// Only host access calls it, and the handle must not outlive the lease. Its key
// names the lease's conversation, device and cwd; offline handles follow reconnects.
func (d *Devices) ConversationHost(
	device webapi.DeviceID,
	files *FileLease,
	cwd string,
	admission remotehost.Admission,
) *remotehost.Host {
	return d.host(
		device,
		host.Key("conversation "+string(files.Conversation())+" "+string(device)+" "+cwd),
		cwd,
		admission,
	)
}

// MachineHost makes the Cloud's Host starting in home, whose operations take the
// Cloud's admission. Machine access touches no conversation's files.
func (d *Devices) MachineHost(device webapi.DeviceID, home string, admission remotehost.Admission) *remotehost.Host {
	return d.host(device, host.Key("machine "+string(device)+" "+home), home, admission)
}

// DeviceAccess returns a Host only while its runner is connected, otherwise nil.
// The caller must own device. This touches no conversation's files, takes no gate
// and never wakes a stopped Cloud.
func (d *Devices) DeviceAccess(device webapi.DeviceID) *remotehost.Host {
	if !d.Online(device) {
		return nil
	}
	return d.host(device, host.Key("device "+string(device)+" /"), "/", nil)
}

// Bind publishes the new connection immediately; sends queue until Serve starts.
// The caller first waits for Settled and refuses a duplicate live connection.
// It transfers driver to the returned Serving and must run Serve or Close.
func (d *Devices) Bind(
	device webapi.DeviceID,
	link *remotehost.Link,
	driver *remotehost.LinkDriver,
	seen *LastSeen,
) *Serving {
	slot := d.slot(device, true)
	slot.publish(remotehost.DeviceLink{Link: link})
	slog.Info("runner connected", "device", device)
	return &Serving{slot: slot, device: device, link: link, driver: driver, seen: seen}
}

// DTO returns the device as the web app sees it, including online state and installs.
func (d *Devices) DTO(device database.DeviceRecord) webapi.DeviceDTO {
	var home *string
	if value, ok := d.Home(device.ID); ok {
		home = &value
	}
	installs := []runnerwire.Install{}
	if link := d.Link(device.ID); link != nil {
		installs = link.Installs()
	}
	return webapi.DeviceDTO{
		ID:         device.ID,
		Kind:       device.Kind,
		Name:       device.Name,
		Platform:   device.Platform,
		ClaimedAt:  device.ClaimedAt,
		LastSeenAt: device.LastSeenAt,
		Online:     d.Online(device.ID),
		Home:       home,
		Installs:   installs,
	}
}

// DeviceList returns user's paired devices oldest first, then its Cloud if created.
func (d *Devices) DeviceList(
	ctx context.Context,
	control *database.ControlService,
	user webapi.UserID,
) ([]webapi.DeviceDTO, error) {
	devices, err := control.PairedDevices(ctx, user)
	if err != nil {
		return nil, err
	}
	cloud, err := control.ManagedDevice(ctx, user)
	if err != nil {
		return nil, err
	}
	if cloud != nil {
		devices = append(devices, *cloud)
	}
	result := make([]webapi.DeviceDTO, 0, len(devices))
	for _, device := range devices {
		result = append(result, d.DTO(device))
	}
	return result, nil
}

// Disconnect ends device's connection, whose runner then reconnects.
func (d *Devices) Disconnect(device webapi.DeviceID, reason string) {
	if link := d.Link(device); link != nil {
		link.Disconnect(reason)
	}
}

// Revoke ends a revoked device's connection and tells its runner to stop for good.
func (d *Devices) Revoke(device webapi.DeviceID) { d.Disconnect(device, "device revoked") }

// DisconnectAll ends every connection for reason. The connection owners join Serve.
func (d *Devices) DisconnectAll(reason string) {
	d.mu.Lock()
	slots := make([]*deviceSlot, 0, len(d.slots))
	for _, slot := range d.slots {
		slots = append(slots, slot)
	}
	d.mu.Unlock()
	for _, slot := range slots {
		current, _ := slot.snapshot()
		if current.Link != nil && !current.Link.IsClosed() {
			current.Link.Disconnect(reason)
		}
	}
}

// Serving owns a bound connection. Its adopting task calls Serve once and joins it
// at shutdown; if adoption fails before Serve, it calls Close instead.
type Serving struct {
	slot   *deviceSlot
	device webapi.DeviceID
	link   *remotehost.Link
	driver *remotehost.LinkDriver
	seen   *LastSeen
}

// Serve owns socket until either end closes, joins connection work, marks the
// device offline and records last seen. It sends a revoked refusal when needed.
// The socket is closed on every exit. The result describes why the link ended.
func (s *Serving) Serve(ctx context.Context, socket *Socket) remotehost.LinkEnd {
	defer s.finished(context.WithoutCancel(ctx))
	defer socket.Release()
	end := s.run(ctx, socket, socket)
	if end.Kind == remotehost.LinkDisconnected && end.Reason == "device revoked" {
		// A runner that went away needs no refusal.
		_ = Send(ctx, socket, &runnerwire.HelloError{Code: runnerwire.HelloErrorCodeRevoked, Reason: "device revoked"})
	}
	// Complete the socket close handshake before releasing its device slot.
	_ = socket.Close(websocket.StatusNormalClosure, "")
	return end
}

// Close releases a bound connection that has not started serving. It marks the
// device offline and records last seen. It must not run concurrently with Serve.
func (s *Serving) Close(ctx context.Context) {
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	s.TestingServe(cancelled, stoppedFrames{}, stoppedFrames{})
}

// TestingServe is the bridge for runnerstest's socket-free connection fixture.
// It serves with the same slot and last-seen cleanup as Serve. Source and sink
// must unblock on cancellation; their resources remain the caller's.
func (s *Serving) TestingServe(
	ctx context.Context,
	incoming remotehost.FrameSource,
	outgoing remotehost.FrameSink,
) remotehost.LinkEnd {
	defer s.finished(context.WithoutCancel(ctx))
	return s.run(ctx, incoming, outgoing)
}

// Send writes one message on a runner socket that nothing else writes to yet,
// such as the answer to its hello. The caller retains socket ownership.
func Send(ctx context.Context, socket *Socket, message runnerwire.Inbound) error {
	frame, err := runnerwire.Encode(message)
	if err != nil {
		return err
	}
	return socket.Send(ctx, frame)
}

// LastSeen records when a runner was connected and marks the owner's device pages.
// Construct it with NewLastSeen; it may be shared between connection owners.
type LastSeen struct {
	control *database.ControlService
	marks   pagesync.UserMarks
}

// NewLastSeen selects the control database and the owner's page marks.
func NewLastSeen(control *database.ControlService, marks pagesync.UserMarks) *LastSeen {
	return &LastSeen{control: control, marks: marks}
}

// Touch records that device's runner was connected just now and marks its pages.
// A storage error is logged: the displayed time decides no admission or ownership.
func (s *LastSeen) Touch(ctx context.Context, device webapi.DeviceID) {
	if err := s.control.TouchDeviceSeen(ctx, device); err != nil {
		slog.Warn("last-seen time not recorded", "device", device, "error", err)
	}
	s.marks.Mark(pagesync.Part{Kind: pagesync.Devices})
}

// slot finds a device connection, creating its retained slot only for Host handles
// and connection owners, never for a read of an unknown device.
func (d *Devices) slot(device webapi.DeviceID, create bool) *deviceSlot {
	d.mu.Lock()
	defer d.mu.Unlock()
	if d.slots == nil && create {
		d.slots = make(map[webapi.DeviceID]*deviceSlot)
	}
	slot := d.slots[device]
	if slot == nil && create {
		slot = &deviceSlot{changed: make(chan struct{})}
		d.slots[device] = slot
	}
	return slot
}

// snapshot pairs a device's immutable connection with its change notification.
func (s *deviceSlot) snapshot() (remotehost.DeviceLink, <-chan struct{}) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.current, s.changed
}

// publish makes a bound runner available before waking connection waiters.
func (s *deviceSlot) publish(current remotehost.DeviceLink) {
	s.mu.Lock()
	old := s.changed
	s.changed = make(chan struct{})
	s.current = current
	s.mu.Unlock()
	close(old)
}

// host follows the device's connection slot without taking conversation admission.
func (d *Devices) host(
	device webapi.DeviceID,
	key host.Key,
	cwd string,
	admission remotehost.Admission,
) *remotehost.Host {
	slot := d.slot(device, true)
	return remotehost.NewHost(key, cwd, func() remotehost.DeviceLink {
		current, _ := slot.snapshot()
		return current
	}, admission)
}

// finished marks only this connection offline before updating its owner's pages.
func (s *Serving) finished(ctx context.Context) {
	identity := s.link.Identity()
	s.slot.mu.Lock()
	var old chan struct{}
	if s.slot.current.Link == s.link {
		old = s.slot.changed
		s.slot.changed = make(chan struct{})
		s.slot.current = remotehost.DeviceLink{Last: &identity}
	}
	s.slot.mu.Unlock()
	if old != nil {
		close(old)
	}
	s.seen.Touch(ctx, s.device)
}

// run owns install notifications for exactly the lifetime of the runner driver.
func (s *Serving) run(
	ctx context.Context,
	incoming remotehost.FrameSource,
	outgoing remotehost.FrameSink,
) remotehost.LinkEnd {
	watchCtx, cancel := context.WithCancel(ctx)
	_, changed := s.link.WatchInstalls()
	done := make(chan struct{})
	go func() {
		defer close(done)
		for {
			select {
			case <-watchCtx.Done():
				return
			case <-changed:
				s.seen.marks.Mark(pagesync.Part{Kind: pagesync.Devices})
				_, changed = s.link.WatchInstalls()
			}
		}
	}()
	end := s.driver.Serve(ctx, incoming, outgoing)
	cancel()
	<-done
	if end.Kind == remotehost.LinkRefused {
		slog.Warn("runner connection closed: "+end.Reason, "device", s.device)
	} else {
		slog.Info("runner connection ended: "+end.Reason, "device", s.device)
	}
	return end
}

// stoppedFrames ends an adopted runner that never acquired its socket.
type stoppedFrames struct{}

// Receive reports EOF for a connection that never started serving.
func (stoppedFrames) Receive(context.Context) ([]byte, error) { return nil, io.EOF }

// Send reports EOF for a connection that never started serving.
func (stoppedFrames) Send(context.Context, []byte) error { return io.EOF }
