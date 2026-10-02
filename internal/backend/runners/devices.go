package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// Devices holds the user's devices and their connection slots. Its zero value
// is ready to use. Methods synchronize slot access; the shard serializes adoption
// decisions (Settled, duplicate refusal, and Bind). Do not copy it.
type Devices struct{}

// Link returns the device's live connection, or nil while offline.
func (d *Devices) Link(device webapi.DeviceID) *remotehost.Link { panic("not written: b-runners") }

// Online reports whether the device has a live runner connection.
func (d *Devices) Online(device webapi.DeviceID) bool { panic("not written: b-runners") }

// UntilOnline waits until a live connection serves device, as a Cloud boot does.
func (d *Devices) UntilOnline(ctx context.Context, device webapi.DeviceID) error {
	panic("not written: b-runners")
}

// Settled waits for a closing connection to finish and mark its slot offline.
func (d *Devices) Settled(ctx context.Context, device webapi.DeviceID) error {
	panic("not written: b-runners")
}

// Home returns the runner's last reported home, or false before its first connection.
func (d *Devices) Home(device webapi.DeviceID) (string, bool) { panic("not written: b-runners") }

// ConversationHost makes the conversation's Host for the operation holding files.
// Only host access calls it, and the handle must not outlive the lease. Its key
// names the lease's conversation, device and cwd; offline handles follow reconnects.
func (d *Devices) ConversationHost(device webapi.DeviceID, files *FileLease, cwd string, admission remotehost.Admission) *remotehost.Host {
	panic("not written: b-runners")
}

// MachineHost makes the Cloud's Host starting in home, whose operations take the
// Cloud's admission. Machine access touches no conversation's files.
func (d *Devices) MachineHost(device webapi.DeviceID, home string, admission remotehost.Admission) *remotehost.Host {
	panic("not written: b-runners")
}

// DeviceAccess returns a Host only while its runner is connected, otherwise nil.
// The caller must own device. This touches no conversation's files, takes no gate
// and never wakes a stopped Cloud.
func (d *Devices) DeviceAccess(device webapi.DeviceID) *remotehost.Host {
	panic("not written: b-runners")
}

// Bind publishes the new connection immediately; sends queue until Serve starts.
// The caller first waits for Settled and refuses a duplicate live connection.
// It transfers driver to the returned Serving and must run Serve or Close.
func (d *Devices) Bind(device webapi.DeviceID, link *remotehost.Link, driver *remotehost.LinkDriver, seen *LastSeen) *Serving {
	panic("not written: b-runners")
}

// DTO returns the device as the web app sees it, including online state and installs.
func (d *Devices) DTO(device database.DeviceRecord) webapi.DeviceDTO { panic("not written: b-runners") }

// DeviceList returns user's paired devices oldest first, then its Cloud if created.
func (d *Devices) DeviceList(ctx context.Context, control *database.ControlService, user webapi.UserID) ([]webapi.DeviceDTO, error) {
	panic("not written: b-runners")
}

// Disconnect ends device's connection, whose runner then reconnects.
func (d *Devices) Disconnect(device webapi.DeviceID, reason string) { panic("not written: b-runners") }

// Revoke ends a revoked device's connection and tells its runner to stop for good.
func (d *Devices) Revoke(device webapi.DeviceID) { panic("not written: b-runners") }

// DisconnectAll ends every connection for reason. The connection owners join Serve.
func (d *Devices) DisconnectAll(reason string) { panic("not written: b-runners") }

// Serving owns a bound connection. Its adopting task calls Serve once and joins it
// at shutdown; if adoption fails before Serve, it calls Close instead.
type Serving struct{}

// Serve owns socket until either end closes, joins connection work, marks the
// device offline and records last seen. It sends a revoked refusal when needed.
// The socket is closed on every exit. The result describes why the link ended.
func (s *Serving) Serve(ctx context.Context, socket *websocket.Conn) remotehost.LinkEnd {
	panic("not written: b-runners")
}

// Close releases a bound connection that has not started serving. It marks the
// device offline and records last seen. It must not run concurrently with Serve.
func (s *Serving) Close(ctx context.Context) { panic("not written: b-runners") }

// TestingServe is the bridge for runnerstest's socket-free connection fixture.
// It serves with the same slot and last-seen cleanup as Serve. Source and sink
// must unblock on cancellation; their resources remain the caller's.
func (s *Serving) TestingServe(ctx context.Context, incoming remotehost.FrameSource, outgoing remotehost.FrameSink) remotehost.LinkEnd {
	panic("not written: b-runners")
}

// Send writes one message on a runner socket that nothing else writes to yet,
// such as the answer to its hello. The caller retains socket ownership.
func Send(ctx context.Context, socket *websocket.Conn, message runnerwire.Inbound) error {
	panic("not written: b-runners")
}

// LastSeen records when a runner was connected and marks the owner's device pages.
// Construct it with NewLastSeen; it may be shared between connection owners.
type LastSeen struct{}

// NewLastSeen selects the control database and the owner's page marks.
func NewLastSeen(control *database.ControlService, marks pagesync.UserMarks) *LastSeen {
	panic("not written: b-runners")
}

// Touch records that device's runner was connected just now and marks its pages.
// A storage error is logged: the displayed time decides no admission or ownership.
func (s *LastSeen) Touch(ctx context.Context, device webapi.DeviceID) {
	panic("not written: b-runners")
}
