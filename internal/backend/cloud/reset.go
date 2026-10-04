package cloud

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/webapi"
)

// Reset returns a running or ready id, resumes a failed id on its original base,
// or admits a new reset. Accepted work belongs to Cloud, not the caller.
func Reset(ctx context.Context, s Shard, id webapi.OperationID) (database.ManagedOperation, error) {
	c := s.Cloud()
	c.mu.Lock()
	stopped := c.stopped || c.ctx.Err() != nil
	c.mu.Unlock()
	if stopped {
		return database.ManagedOperation{}, &Error{Kind: Closed}
	}
	device, err := Device(ctx, s)
	if err != nil {
		return database.ManagedOperation{}, storageFailed(err)
	}
	m, err := loadMachine(ctx, s, device)
	if err != nil {
		return database.ManagedOperation{}, storageFailed(err)
	}
	c.mu.Lock()
	running, ok, err := runningResetLocked(m, id)
	c.mu.Unlock()
	if err != nil {
		return database.ManagedOperation{}, err
	}
	if ok {
		return running, nil
	}
	stored, found, err := cloudRecords(s).ManagedOperation(ctx, device.ID, id)
	if err != nil {
		return database.ManagedOperation{}, storageFailed(err)
	}
	if found && stored.Phase == webapi.ResetPhaseReady {
		return stored, nil
	}
	var base machinewire.BaseVersion
	if found {
		base = stored.BaseVersion
	} else {
		base, err = Call(ctx, s.CloudServices().Machines, machinewire.CurrentBaseVersionParams{})
		if err != nil {
			return database.ManagedOperation{}, failed(err)
		}
	}
	return admitReset(c, s, m, id, base)
}

// runningResetLocked reads the admitted reset under the shard mutex.
func runningResetLocked(m *machine, id webapi.OperationID) (database.ManagedOperation, bool, error) {
	if m.reset == nil {
		return database.ManagedOperation{}, false, nil
	}
	if m.operation.ID != id {
		return database.ManagedOperation{}, false, &Error{Kind: Resetting}
	}
	return *m.operation, true, nil
}

// resetSteps writes intent before each disk step and holds affected conversations.
func resetSteps(
	ctx context.Context,
	s Shard,
	m *machine,
	op database.ManagedOperation,
	previous, retirement *transition,
) error {
	if err := s.CloudStopped(ctx, m.device.ID); err != nil {
		return failed(err)
	}
	if err := recordPhase(ctx, s, m, op, webapi.ResetPhaseStopping, nil); err != nil {
		return err
	}
	if previous != nil {
		// A failed boot/save still must finish before resetting disks.
		_ = previous.wait(ctx)
	}
	if retirement != nil {
		// Its phase recheck yields to this reset.
		_ = retirement.wait(ctx)
	}
	uses, err := cloudUses(ctx, s)
	if err != nil {
		return storageFailed(err)
	}
	held, err := holdReset(ctx, s, uses, s.CloudServices().Tuning.ResetHold)
	if err != nil {
		return err
	}
	defer releaseHolds(held)
	flush(ctx, s, m.device.ID)
	s.Devices().Disconnect(m.device.ID, "Cloud is resetting")
	wait, cancel := context.WithTimeout(ctx, s.CloudServices().Tuning.ResetHold)
	reserved, err := m.gate.Reserve(wait)
	cancel()
	if err != nil {
		//nolint:staticcheck // Product text, shown to the user as it is.
		return failed(errors.New("The Cloud's operations did not end for the reset"))
	}
	defer reserved.Release()
	return rebuildReset(ctx, s, m, op)
}

// finishReset records the result then atomically publishes the final reset and lifecycle state.
func finishReset(
	ctx context.Context,
	s Shard,
	m *machine,
	op database.ManagedOperation,
	t *transition,
	result error,
) {
	op.Phase = webapi.ResetPhaseReady
	if result != nil {
		if err := save(ctx, s, m); err != nil {
			slog.Warn("a Cloud whose reset failed was not saved", "error", err)
		}
		text := result.Error()
		op.Phase = webapi.ResetPhaseFailed
		op.Error = &text
	}
	recorded := cloudRecords(s).PutManagedOperation(ctx, m.device.ID, op)
	c := s.Cloud()
	c.mu.Lock()
	m.operation = &op
	m.reset = nil
	var permit *Permit
	if result == nil {
		m.phase = webapi.CloudStateRunning
		m.failure = nil
		m.started = time.Now()
		m.checkpoint = m.started
	} else {
		m.phase = webapi.CloudStateOff
		m.failure = op.Error
		permit = m.permit
		m.permit = nil
	}
	c.mu.Unlock()
	if permit != nil {
		permit.Release()
	}
	m.mark()
	if result == nil {
		startSchedules(s, m)
	}
	if result == nil {
		result = storageFailed(recorded)
	}
	t.err = result
	close(t.done)
}

// recordPhase commits a reset phase before publishing it, including a failed write.
func recordPhase(
	ctx context.Context,
	s Shard,
	m *machine,
	op database.ManagedOperation,
	phase webapi.ResetPhase,
	failure *string,
) error {
	op.Phase = phase
	op.Error = failure
	err := cloudRecords(s).PutManagedOperation(ctx, m.device.ID, op)
	c := s.Cloud()
	c.mu.Lock()
	m.operation = &op
	c.mu.Unlock()
	m.mark()
	return storageFailed(err)
}

// RecoverResets reconciles before serving, removes stale exposes and recovers
// each unfinished reset's disks. It records failure for a retry without booting.
func RecoverResets(ctx context.Context, control *database.ControlService, services *Services) error {
	return recoverResets(ctx, control, services)
}

// resetRecords is startup recovery's durable control boundary.
type resetRecords interface {
	DeleteCloudExposes(context.Context) error
	UnfinishedManagedOperations(context.Context) ([]database.DeviceOperation, error)
	Device(context.Context, webapi.DeviceID) (database.DeviceRecord, bool, error)
	AnnounceCloudReset(context.Context, webapi.UserID, webapi.OperationID) error
	PutManagedOperation(context.Context, webapi.DeviceID, database.ManagedOperation) error
}

// recoverResets orders manager reconciliation and durable reset recovery before serving.
func recoverResets(ctx context.Context, control resetRecords, services *Services) error {
	if _, err := Call(ctx, services.Machines, machinewire.ReconcileParams{}); err != nil {
		return err
	}
	if err := control.DeleteCloudExposes(ctx); err != nil {
		return err
	}
	operations, err := control.UnfinishedManagedOperations(ctx)
	if err != nil {
		return err
	}
	for _, pair := range operations {
		device, found, err := control.Device(ctx, pair.Device)
		if err != nil {
			return err
		}
		if !found {
			return fmt.Errorf("a reset names the device %s, which no longer exists", pair.Device)
		}
		op := pair.Operation
		if _, err := Call(
			ctx,
			services.Machines,
			machinewire.ResetParams{
				DeviceID:    string(pair.Device),
				OperationID: string(op.ID),
				BaseVersion: string(op.BaseVersion),
			},
		); err != nil {
			return err
		}
		if err := control.AnnounceCloudReset(ctx, device.User, op.ID); err != nil {
			return err
		}
		text := "Reset disks recovered; retry to start Cloud"
		op.Phase = webapi.ResetPhaseFailed
		op.Error = &text
		if err := control.PutManagedOperation(ctx, pair.Device, op); err != nil {
			return err
		}
	}
	return nil
}

// admitReset transfers the machine permit and previous work to an owned reset.
func admitReset(
	c *Cloud,
	s Shard,
	m *machine,
	id webapi.OperationID,
	base machinewire.BaseVersion,
) (database.ManagedOperation, error) {
	operation := database.ManagedOperation{ID: id, BaseVersion: base, Phase: webapi.ResetPhaseStopping}
	capacity := s.CloudServices().Capacity
	c.mu.Lock()
	if c.stopped || c.ctx.Err() != nil {
		c.mu.Unlock()
		return database.ManagedOperation{}, &Error{Kind: Closed}
	}
	running, ok, err := runningResetLocked(m, id)
	if ok || err != nil {
		c.mu.Unlock()
		return running, err
	}
	if m.permit == nil {
		// Lock order is shard then capacity. Capacity never calls into a shard.
		m.permit = capacity.TryTake()
		if m.permit == nil {
			c.mu.Unlock()
			return database.ManagedOperation{}, &Error{Kind: AtCapacity}
		}
	}
	previous := m.transition
	retirement := m.retirement
	cancel := m.schedules
	m.schedules = nil
	m.phase = webapi.CloudStateResetting
	m.operation = &operation
	t := &transition{done: make(chan struct{})}
	m.reset = t
	c.workers.Add(1)
	c.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	m.mark()
	go func() {
		defer c.workers.Done()
		result := runTransition(
			context.WithoutCancel(c.ctx),
			func(ctx context.Context) error { return resetSteps(ctx, s, m, operation, previous, retirement) },
		)
		finishReset(context.WithoutCancel(c.ctx), s, m, operation, t, result)
	}()
	return operation, nil
}

// rebuildReset persists saving, rebuilding and booting intent before each disk step.
func rebuildReset(ctx context.Context, s Shard, m *machine, op database.ManagedOperation) error {
	if err := recordPhase(ctx, s, m, op, webapi.ResetPhaseSaving, nil); err != nil {
		return err
	}
	if err := save(ctx, s, m); err != nil {
		return err
	}
	if err := recordPhase(ctx, s, m, op, webapi.ResetPhaseRebuilding, nil); err != nil {
		return err
	}
	_, err := Call(
		ctx,
		s.CloudServices().Machines,
		machinewire.ResetParams{
			DeviceID:    string(m.device.ID),
			OperationID: string(op.ID),
			BaseVersion: string(op.BaseVersion),
		},
	)
	if err != nil {
		return failed(err)
	}
	if err := cloudRecords(s).AnnounceCloudReset(ctx, s.User(), op.ID); err != nil {
		return storageFailed(err)
	}
	c := s.Cloud()
	c.mu.Lock()
	m.deaths = nil
	c.mu.Unlock()
	if err := recordPhase(ctx, s, m, op, webapi.ResetPhaseBooting, nil); err != nil {
		return err
	}
	return boot(ctx, s, m)
}
