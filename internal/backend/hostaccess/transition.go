package hostaccess

import (
	"context"
	"errors"
	"log/slog"
	"sync"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapiproto"
)

// TransitionHold holds file reservation, closed transfers and tree reservation.
// The owner defers Release. No Host operation may reenter its file gate.
type TransitionHold struct {
	once      sync.Once
	files     *gates.Reservation
	transfers *TransfersClosed
	tree      *gates.Reservation
	done      func()
}

// Release releases files, reopens transfers after all closing holds end, then
// releases the tree reservation. It is idempotent and does not wait.
func (h *TransitionHold) Release() {
	h.once.Do(func() {
		if h.files != nil {
			h.files.Release()
		}
		if h.transfers != nil {
			h.transfers.Release()
		}
		if h.tree != nil {
			h.tree.Release()
		}
		if h.done != nil {
			h.done()
		}
	})
}

// HoldForTransition takes ownership of tree, including on failure. It closes
// transfers atomically, ends and joins their admissions outside the mutex,
// then tries the file reservation. Other file work answers busy.
func HoldForTransition(
	ctx context.Context,
	shard HostShard,
	id webapiproto.ConversationID,
	tree *gates.Reservation,
) (*TransitionHold, error) {
	hold := &TransitionHold{tree: tree}
	var err error
	hold.done, err = shard.Conversations().begin()
	if err != nil {
		hold.Release()
		return nil, err
	}
	slot := shard.Conversations().Slot(id)
	hold.transfers, err = slot.transfers.Close(ctx)
	if err != nil {
		hold.Release()
		return nil, err
	}
	hold.files = slot.files.Gate().TryReserve()
	if hold.files == nil {
		hold.Release()
		return nil, &ChangeError{Kind: ChangeTurnInFlight}
	}
	return hold, nil
}

// Commit applies a held record change. Once commit starts it finishes with
// shard-owned bookkeeping even if the requester leaves.
func Commit(ctx context.Context, shard HostShard, id webapiproto.ConversationID, change database.RecordChange) error {
	err := shard.Control().ChangeConversation(context.WithoutCancel(ctx), id, change)
	switch {
	case err == nil:
		return nil
	case errors.Is(err, database.ErrConversationNotFound):
		return &ChangeError{Kind: ChangeNotFound}
	case errors.Is(err, database.ErrArchived):
		return &ChangeError{Kind: ChangeArchived}
	case errors.Is(err, database.ErrNotAttached):
		return &ChangeError{Kind: ChangeNotAttached}
	case errors.Is(err, database.ErrNameTaken):
		return &ChangeError{Kind: ChangeNameTaken}
	}
	return &ChangeError{Kind: ChangeStorage, Cause: err}
}

// CheckDestination checks ownership of the workspace or paired destination.
func CheckDestination(
	ctx context.Context,
	shard HostShard,
	record database.ConversationRecord,
	to webapiproto.ConversationTarget,
) error {
	switch target := to.(type) {
	case *webapiproto.ConversationTargetWorkspace:
		workspace, found, err := shard.Control().Workspace(ctx, target.WorkspaceID)
		if err != nil {
			return &ChangeError{Kind: ChangeStorage, Cause: err}
		}
		if !found || workspace.User != record.Owner {
			return &ChangeError{Kind: ChangeWorkspaceNotFound}
		}
	case *webapiproto.ConversationTargetDevice:
		device, ok, err := shard.Control().Device(ctx, target.DeviceID)
		if err != nil {
			return &ChangeError{Kind: ChangeStorage, Cause: err}
		}
		if !ok || device.User != record.Owner || device.Kind != webapiproto.DeviceKindUser {
			return &ChangeError{Kind: ChangeDeviceNotFound}
		}
	case *webapiproto.ConversationTargetCloud:
	}
	return nil
}

// SwitchTarget releases the departed device and commits against the expected
// selection while the caller holds a TransitionHold. A successful write
// restarts idle tracking even if the requester left during commit.
func SwitchTarget(
	ctx context.Context,
	shard HostShard,
	expected database.ConversationRecord,
	to webapiproto.ConversationTarget,
) error {
	ctx = context.WithoutCancel(ctx)
	current, found, err := shard.Control().Conversation(ctx, expected.ID)
	if err != nil {
		return &ChangeError{Kind: ChangeStorage, Cause: err}
	}
	if !found {
		return &ChangeError{Kind: ChangeNotFound}
	}
	if current.Archived {
		return &ChangeError{Kind: ChangeArchived}
	}
	from, err := ResolveTarget(ctx, shard, expected)
	if err != nil {
		return &ChangeError{Kind: ChangeStorage, Cause: err}
	}
	reaching := expected
	reaching.Target = to
	destination, err := ResolveTarget(ctx, shard, reaching)
	if err != nil {
		return &ChangeError{Kind: ChangeStorage, Cause: err}
	}
	departed, departs := database.ExecutionDeviceID(from)
	arriving, arrives := database.ExecutionDeviceID(destination)
	ends := database.SwitchEnds{}
	if arrives {
		ends.Arriving = &arriving
	}
	if departs {
		ends.Departed = &database.DepartedHost{Device: departed, Path: database.ExecutionPath(from)}
		if !arrives || departed != arriving {
			releaseOn(ctx, shard, expected.ID, departed)
		}
	}
	won, err := shard.Control().
		SwitchConversationTarget(
			ctx, expected.ID, expected.Target, to,
			database.TargetSwitch{From: from, To: destination}, ends,
		)
	if err != nil {
		return &ChangeError{Kind: ChangeStorage, Cause: err}
	}
	if !won {
		return &ChangeError{Kind: ChangeConflict}
	}
	shard.TrackIdle(expected.ID)
	return nil
}

// Archive releases all reachable Hosts and commits under a TransitionHold.
func Archive(ctx context.Context, shard HostShard, record database.ConversationRecord) error {
	ctx = context.WithoutCancel(ctx)
	if err := ReleaseEverywhere(ctx, shard, record); err != nil {
		return &ChangeError{Kind: ChangeStorage, Cause: err}
	}
	return Commit(ctx, shard, record.ID, &database.RecordArchived{Archived: true})
}

// Detach releases the attached device and commits under a TransitionHold.
func Detach(
	ctx context.Context,
	shard HostShard,
	record database.ConversationRecord,
	device webapiproto.DeviceID,
) error {
	ctx = context.WithoutCancel(ctx)
	attached, err := shard.Control().AttachedHosts(ctx, record.ID)
	if err != nil {
		return &ChangeError{Kind: ChangeStorage, Cause: err}
	}
	for _, bound := range attached {
		if bound.Device == device {
			releaseOn(ctx, shard, record.ID, device)
			break
		}
	}
	return Commit(ctx, shard, record.ID, &database.RecordDetach{Device: device})
}

// ReleaseEverywhere sends lifecycle release to connected bound devices, waking
// nothing. Only reading the bound Hosts fails it; runner release failures are
// logged and do not fail the transition or idle cleanup.
func ReleaseEverywhere(ctx context.Context, shard HostShard, record database.ConversationRecord) error {
	target, err := ResolveTarget(ctx, shard, record)
	if err != nil {
		return err
	}
	hosts, err := reachableHosts(ctx, shard, record, target)
	if err != nil {
		return err
	}
	for _, bound := range hosts {
		releaseOn(ctx, shard, record.ID, bound.Device)
	}
	return nil
}

// releaseOn performs lifecycle release only over an existing runner connection.
func releaseOn(
	ctx context.Context,
	shard HostShard,
	conversation webapiproto.ConversationID,
	device webapiproto.DeviceID,
) {
	link := shard.Devices().Link(device)
	if link == nil {
		return
	}
	if err := link.ReleaseConversation(ctx, string(conversation)); err != nil {
		slog.Warn("the conversation release failed", "device", device, "conversation", conversation, "error", err)
	}
}
