package hostaccess

//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
)

// TransitionHold holds file reservation, closed transfers and tree reservation.
// The owner defers Release. No Host operation may reenter its file gate.
type TransitionHold struct{}

// Release releases files, reopens transfers after all closing holds end, then
// releases the tree reservation. It is idempotent and does not wait.
func (h *TransitionHold) Release() { panic("not written: b-hostaccess") }

// HoldForTransition takes ownership of tree, including on failure. It closes
// transfers atomically, ends and joins their admissions outside the mutex,
// then tries the file reservation. Other file work answers busy.
func HoldForTransition(ctx context.Context, shard HostShard, id webapi.ConversationID, tree *gates.Reservation) (*TransitionHold, error) {
	panic("not written: b-hostaccess")
}

// Commit applies a held record change. Once commit starts it finishes with
// shard-owned bookkeeping even if the requester leaves.
func Commit(ctx context.Context, shard HostShard, id webapi.ConversationID, change database.RecordChange) error {
	panic("not written: b-hostaccess")
}

// CheckDestination checks ownership of the workspace or paired destination.
func CheckDestination(ctx context.Context, shard HostShard, record database.ConversationRecord, to webapi.ConversationTarget) error {
	panic("not written: b-hostaccess")
}

// SwitchTarget releases the departed device and commits against the expected
// selection while the caller holds a TransitionHold. A successful write
// restarts idle tracking even if the requester left during commit.
func SwitchTarget(ctx context.Context, shard HostShard, expected database.ConversationRecord, to webapi.ConversationTarget) error {
	panic("not written: b-hostaccess")
}

// Archive releases all reachable Hosts and commits under a TransitionHold.
func Archive(ctx context.Context, shard HostShard, record database.ConversationRecord) error {
	panic("not written: b-hostaccess")
}

// Detach releases the attached device and commits under a TransitionHold.
func Detach(ctx context.Context, shard HostShard, record database.ConversationRecord, device webapi.DeviceID) error {
	panic("not written: b-hostaccess")
}

// ReleaseEverywhere sends lifecycle release to connected bound devices, waking
// nothing. Only reading the bound Hosts fails it; runner release failures are
// logged and do not fail the transition or idle cleanup.
func ReleaseEverywhere(ctx context.Context, shard HostShard, record database.ConversationRecord) error {
	panic("not written: b-hostaccess")
}
