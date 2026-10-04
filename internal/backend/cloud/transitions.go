package cloud

import (
	"context"
	"errors"
	"log/slog"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/machineproto"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/webapiproto"
)

// boot rotates the Cloud credential and starts a sandbox, saving on failure.
func boot(ctx context.Context, s Shard, m *machine) error {
	err := startSandbox(ctx, s, m)
	if err != nil {
		if _, saved := Call(
			ctx,
			s.CloudServices().Machines,
			machineproto.HibernateParams{DeviceID: string(m.device.ID)},
		); saved != nil {
			slog.Warn("a Cloud whose boot failed was not saved", "error", saved)
		}
		s.Devices().Disconnect(m.device.ID, "Cloud boot failed")
	}
	return err
}

// startSandbox waits for authenticated runner readiness after the manager's wake.
func startSandbox(ctx context.Context, s Shard, m *machine) error {
	token := runners.NewDeviceToken()
	if err := cloudRecords(s).RotateDeviceToken(ctx, m.device.ID, database.HashToken(token.Expose())); err != nil {
		return storageFailed(err)
	}
	backend, ok := s.PublicURL().URL()
	if !ok {
		//nolint:staticcheck // Product text, shown to the user as it is.
		return failed(errors.New("The backend does not listen yet"))
	}
	_, err := Call(
		ctx,
		s.CloudServices().Machines,
		machineproto.WakeParams{
			DeviceID: string(m.device.ID),
			Boot:     runnerproto.ManagedBoot{BackendURL: backend, DeviceToken: token},
		},
	)
	if err != nil {
		return failed(err)
	}
	// The shard's lifetime, not the requester's, stops readiness at shutdown.
	connected, cancel := context.WithTimeout(s.Cloud().ctx, s.CloudServices().Tuning.RunnerConnection)
	defer cancel()
	err = s.Devices().UntilOnline(connected, m.device.ID)
	if s.Cloud().ctx.Err() != nil {
		return &Error{Kind: Closed}
	}
	if err != nil {
		return failed(errors.New("Cloud boot timeout: its runner did not connect"))
	}
	return nil
}

// finishBoot publishes boot's outcome unless a reset has taken its permit.
func finishBoot(s Shard, m *machine, err error) {
	c := s.Cloud()
	c.mu.Lock()
	if m.phase != webapiproto.CloudStateBooting {
		c.mu.Unlock()
		return
	}
	var permit *Permit
	if err != nil {
		m.phase = webapiproto.CloudStateOff
		text := err.Error()
		m.failure = &text
		permit = m.permit
		m.permit = nil
	} else {
		m.phase = webapiproto.CloudStateRunning
		m.failure = nil
		m.started = time.Now()
		m.checkpoint = m.started
	}
	c.mu.Unlock()
	if permit != nil {
		permit.Release()
	}
	m.mark()
	if err == nil {
		startSchedules(s, m)
	}
}

// recoverMachine preserves a running sandbox's token and work while its runner reconnects.
func recoverMachine(ctx context.Context, s Shard, m *machine) error {
	state, err := Call(
		ctx,
		s.CloudServices().Machines,
		machineproto.RuntimeStateParams{DeviceID: string(m.device.ID)},
	)
	if err != nil {
		return failed(err)
	}
	if state == machineproto.RuntimeStateRunning {
		wait, cancel := context.WithTimeout(s.Cloud().ctx, s.CloudServices().Tuning.RunnerConnection)
		defer cancel()
		if err := s.Devices().UntilOnline(wait, m.device.ID); err != nil {
			if s.Cloud().ctx.Err() != nil {
				return &Error{Kind: Closed}
			}
			return failed(errors.New("Cloud runner reconnect timeout"))
		}
		return nil
	}
	c := s.Cloud()
	c.mu.Lock()
	if m.phase != webapiproto.CloudStateRunning {
		c.mu.Unlock()
		return nil
	}
	cancel := m.schedules
	m.schedules = nil
	m.phase = webapiproto.CloudStateBooting
	c.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	m.mark()
	if err := s.CloudStopped(ctx, m.device.ID); err != nil {
		return failed(err)
	}
	return boot(ctx, s, m)
}

// hibernate saves under a reservation held by its caller, joining an earlier boot.
func hibernate(ctx context.Context, s Shard, m *machine) error {
	c := s.Cloud()
	c.mu.Lock()
	previous := m.transition
	c.mu.Unlock()
	if previous != nil {
		// A failed boot is already off; either outcome must settle before saving.
		_ = previous.wait(context.WithoutCancel(ctx))
	}
	c.mu.Lock()
	if m.phase != webapiproto.CloudStateRunning {
		c.mu.Unlock()
		return nil
	}
	cancel := m.schedules
	m.schedules = nil
	m.phase = webapiproto.CloudStateSaving
	t := &transition{done: make(chan struct{})}
	m.transition = t
	// The caller owns and joins this synchronous transition; reset can join it too.
	c.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	m.mark()
	durable := context.WithoutCancel(ctx)
	err := runTransition(durable, func(ctx context.Context) error {
		notice := s.CloudStopped(ctx, m.device.ID)
		if err := save(ctx, s, m); err != nil {
			return err
		}
		return failed(notice)
	})
	c.mu.Lock()
	var permit *Permit
	if m.phase == webapiproto.CloudStateSaving {
		m.phase = webapiproto.CloudStateOff
		permit = m.permit
		m.permit = nil
	}
	if err != nil {
		text := err.Error()
		m.failure = &text
	}
	m.transition = nil
	c.mu.Unlock()
	if permit != nil {
		permit.Release()
	}
	m.mark()
	t.err = err
	close(t.done)
	return err
}

// save flushes best effort, asks the manager to persist storage, and disconnects the runner.
func save(ctx context.Context, s Shard, m *machine) error {
	flush(ctx, s, m.device.ID)
	_, err := Call(ctx, s.CloudServices().Machines, machineproto.HibernateParams{DeviceID: string(m.device.ID)})
	s.Devices().Disconnect(m.device.ID, "Cloud stopped")
	return failed(err)
}

// flush bounds a live runner's filesystem sync without preventing manager-side saves.
func flush(ctx context.Context, s Shard, device webapiproto.DeviceID) {
	link := s.Devices().Link(device)
	if link == nil {
		return
	}
	wait, cancel := context.WithTimeout(ctx, s.CloudServices().Tuning.SyncTimeout)
	defer cancel()
	if err := link.Sync(wait); err != nil {
		if errors.Is(wait.Err(), context.DeadlineExceeded) {
			slog.Warn("the Cloud did not flush its filesystems in time", "device", device)
		} else {
			slog.Warn("the Cloud did not flush its filesystems", "device", device, "error", err)
		}
	}
}
