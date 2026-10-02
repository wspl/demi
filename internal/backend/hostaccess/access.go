package hostaccess

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"sync"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
)

// Conversations owns the user's slots and host-access workers. Its opaque state
// uses the shard mutex; callers hold no shard lock when calling its methods.
// Construct with NewConversations, do not copy, and close before dependencies.
type Conversations struct{}

// NewConversations creates slots and a worker owner under the shard's lifetime
// context and mutex. Request cancellation never abandons an admitted commit.
func NewConversations(ctx context.Context, mu *sync.Mutex) *Conversations {
	panic("not written: b-hostaccess")
}

// Slot returns the stable opaque slot of the canonical conversation ID.
func (c *Conversations) Slot(id webapi.ConversationID) *ConversationSlot {
	panic("not written: b-hostaccess")
}

// EndTransfers closes admission, ends registered transfers and streams, and
// waits for their admission release. Each returned hold must be released.
func (c *Conversations) EndTransfers(ctx context.Context) ([]*TransfersClosed, error) {
	panic("not written: b-hostaccess")
}

// Close stops admission, interrupts streams and transfers, and joins owned
// work, including commits. It must finish before shard dependencies are closed.
func (c *Conversations) Close(ctx context.Context) error { panic("not written: b-hostaccess") }

// ConversationSlot owns one conversation's gates and transfer registrations.
// Its methods return synchronized opaque handles, never mutable shard state.
type ConversationSlot struct{}

// FileGate is the activity gate other work holds or reserves. Its leases make
// no Host; only host access can acquire the private runner file lease.
func (s *ConversationSlot) FileGate() *gates.Activity { panic("not written: b-hostaccess") }

// Streams is the activity of admitted user streams.
func (s *ConversationSlot) Streams() *gates.Activity { panic("not written: b-hostaccess") }

// Transfers is the synchronized registry of transfers and user streams.
func (s *ConversationSlot) Transfers() *TransferSet { panic("not written: b-hostaccess") }

// Settings serializes changes of a conversation's settings.
func (s *ConversationSlot) Settings() *gates.Serial { panic("not written: b-hostaccess") }

// ConversationHost is the Host reached by an admitted operation. Host must not
// escape the admission; Root is where work starts and Home is the reported home.
type ConversationHost struct {
	Host *remotehost.Host
	Root string
	Home *string
}

// Admitted holds the Host, file lease and Cloud admission of one operation.
// Its owner defers Release and uses Host only while the admission is held.
type Admitted struct{ Host ConversationHost }

// Release ends the admission exactly once, releasing files before Cloud.
func (a *Admitted) Release() { panic("not written: b-hostaccess") }

// HostRole says whether a reachable Host is main or attached.
type HostRole uint8

const (
	// Main is the conversation's selected Host.
	Main HostRole = iota
	// Attached is a device attached to the conversation.
	Attached
)

// ReachableHost names a bound device and the directory its shells start in.
type ReachableHost struct {
	Name   string
	Device webapi.DeviceID
	Path   string
	Role   HostRole
}

// OwnedConversation reads the owner's conversation, regardless of ID case.
// Another user's conversation answers as missing.
func OwnedConversation(ctx context.Context, shard HostShard, id webapi.ConversationID) (database.ConversationRecord, error) {
	panic("not written: b-hostaccess")
}

// WithHost runs operation once on the main Host or named bound device, holding
// file and Cloud admission until it returns. Waiting rechecks ownership,
// binding and archive state; dispatched work is never retried.
func WithHost[T any](ctx context.Context, shard HostShard, id webapi.ConversationID, device *webapi.DeviceID, operation func(context.Context, *ConversationHost) (T, error)) (T, error) {
	panic("not written: b-hostaccess")
}

// AdmitHost takes the conversation's Host admission. It releases the file gate
// before waiting for Cloud and repeats every check after reacquiring it.
// The caller must defer Release on success; ctx cancels admission waits.
func AdmitHost(ctx context.Context, shard HostShard, id webapi.ConversationID, device *webapi.DeviceID) (*Admitted, error) {
	panic("not written: b-hostaccess")
}

// ResolveTarget resolves metadata without allocating or starting Cloud.
func ResolveTarget(ctx context.Context, shard HostShard, record database.ConversationRecord) (database.ExecutionTarget, error) {
	panic("not written: b-hostaccess")
}

// ConversationHosts lists the main Host followed by the attached Hosts.
func ConversationHosts(ctx context.Context, shard HostShard, id webapi.ConversationID) ([]ReachableHost, error) {
	panic("not written: b-hostaccess")
}
