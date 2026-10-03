package hostaccess

import (
	"context"
	"log/slog"
	"sync"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
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
func HoldForTransition(ctx context.Context, shard HostShard, id webapi.ConversationID, tree *gates.Reservation) (*TransitionHold, error) {
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
		return nil, &ChangeRefusal{Kind: ChangeTurnInFlight}
	}
	return hold, nil
}

// Commit applies a held record change. Once commit starts it finishes with
// shard-owned bookkeeping even if the requester leaves.
func Commit(ctx context.Context, shard HostShard, id webapi.ConversationID, change database.RecordChange) error {
	outcome, err := shard.Control().ChangeConversation(context.WithoutCancel(ctx), id, change)
	if err != nil {
		return &ChangeRefusal{Kind: ChangeStorage, Cause: err}
	}
	switch outcome {
	case database.ChangeApplied:
		return nil
	case database.ChangeMissing:
		return &ChangeRefusal{Kind: ChangeNotFound}
	case database.ChangeArchived:
		return &ChangeRefusal{Kind: ChangeArchived}
	case database.ChangeNotAttached:
		return &ChangeRefusal{Kind: ChangeNotAttached}
	case database.ChangeNameTaken:
		return &ChangeRefusal{Kind: ChangeNameTaken}
	}
	return nil
}

// CheckDestination checks ownership of the workspace or paired destination.
func CheckDestination(ctx context.Context, shard HostShard, record database.ConversationRecord, to webapi.ConversationTarget) error {
	switch target := to.(type) {
	case *webapi.ConversationTargetWorkspace:
		workspace, err := shard.Control().Workspace(ctx, target.WorkspaceID)
		if err != nil {
			return &ChangeRefusal{Kind: ChangeStorage, Cause: err}
		}
		if workspace == nil || workspace.User != record.Owner {
			return &ChangeRefusal{Kind: ChangeWorkspaceNotFound}
		}
	case *webapi.ConversationTargetDevice:
		device, err := shard.Control().Device(ctx, target.DeviceID)
		if err != nil {
			return &ChangeRefusal{Kind: ChangeStorage, Cause: err}
		}
		if device == nil || device.User != record.Owner || device.Kind != webapi.DeviceKindUser {
			return &ChangeRefusal{Kind: ChangeDeviceNotFound}
		}
	case *webapi.ConversationTargetCloud:
	}
	return nil
}

// SwitchTarget releases the departed device and commits against the expected
// selection while the caller holds a TransitionHold. A successful write
// restarts idle tracking even if the requester left during commit.
func SwitchTarget(ctx context.Context, shard HostShard, expected database.ConversationRecord, to webapi.ConversationTarget) error {
	ctx = context.WithoutCancel(ctx)
	current, err := shard.Control().Conversation(ctx, expected.ID)
	if err != nil {
		return &ChangeRefusal{Kind: ChangeStorage, Cause: err}
	}
	if current == nil {
		return &ChangeRefusal{Kind: ChangeNotFound}
	}
	if current.Archived {
		return &ChangeRefusal{Kind: ChangeArchived}
	}
	from, err := ResolveTarget(ctx, shard, expected)
	if err != nil {
		return &ChangeRefusal{Kind: ChangeStorage, Cause: err}
	}
	reaching := expected
	reaching.Target = to
	destination, err := ResolveTarget(ctx, shard, reaching)
	if err != nil {
		return &ChangeRefusal{Kind: ChangeStorage, Cause: err}
	}
	departed := database.ExecutionDeviceID(from)
	arriving := database.ExecutionDeviceID(destination)
	ends := database.SwitchEnds{Arriving: arriving}
	if departed != nil {
		ends.Departed = &database.DepartedHost{Device: *departed, Path: database.ExecutionPath(from)}
		if arriving == nil || *departed != *arriving {
			releaseOn(ctx, shard, expected.ID, *departed)
		}
	}
	won, err := shard.Control().SwitchConversationTarget(ctx, expected.ID, expected.Target, to, database.TargetSwitch{From: from, To: destination}, ends)
	if err != nil {
		return &ChangeRefusal{Kind: ChangeStorage, Cause: err}
	}
	if !won {
		return &ChangeRefusal{Kind: ChangeConflict}
	}
	shard.TrackIdle(expected.ID)
	return nil
}

// Archive releases all reachable Hosts and commits under a TransitionHold.
func Archive(ctx context.Context, shard HostShard, record database.ConversationRecord) error {
	ctx = context.WithoutCancel(ctx)
	if err := ReleaseEverywhere(ctx, shard, record); err != nil {
		return &ChangeRefusal{Kind: ChangeStorage, Cause: err}
	}
	return Commit(ctx, shard, record.ID, &database.RecordArchived{Archived: true})
}

// Detach releases the attached device and commits under a TransitionHold.
func Detach(ctx context.Context, shard HostShard, record database.ConversationRecord, device webapi.DeviceID) error {
	ctx = context.WithoutCancel(ctx)
	attached, err := shard.Control().AttachedHosts(ctx, record.ID)
	if err != nil {
		return &ChangeRefusal{Kind: ChangeStorage, Cause: err}
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
func releaseOn(ctx context.Context, shard HostShard, conversation webapi.ConversationID, device webapi.DeviceID) {
	link := shard.Devices().Link(device)
	if link == nil {
		return
	}
	if err := link.ReleaseConversation(ctx, string(conversation)); err != nil {
		slog.Warn("the conversation release failed", "device", device, "conversation", conversation, "error", err)
	}
}
