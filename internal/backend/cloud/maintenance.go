package cloud

import (
	"context"
	"log/slog"
	"time"

	"github.com/wspl/demi/internal/backend/idlewatch"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/machineproto"
	"github.com/wspl/demi/internal/webapiproto"
)

// startSchedules registers a running phase's workers before shutdown can join them.
func startSchedules(s Shard, m *machine) {
	c := s.Cloud()
	window, tuning := s.IdleWindow(), s.CloudServices().Tuning
	ctx, cancel := context.WithCancel(c.ctx)
	c.mu.Lock()
	if c.stopped || m.phase != webapiproto.CloudStateRunning || m.schedules != nil {
		c.mu.Unlock()
		cancel()
		return
	}
	m.schedules = cancel
	c.workers.Add(2)
	c.mu.Unlock()
	go func() {
		defer c.workers.Done()
		idlewatch.Watch(ctx, &cloudIdle{s: s, m: m}, window, tuning.Sweep)
	}()
	go func() {
		defer c.workers.Done()
		maintain(ctx, s, m, tuning.Sweep)
	}()
}

// maintain runs rounds on the Cloud sweep, with delayed ticks after a slow round.
func maintain(ctx context.Context, s Shard, m *machine, sweep time.Duration) {
	timer := time.NewTimer(sweep)
	defer timer.Stop()
	next := time.Now().Add(sweep)
	for {
		select {
		case <-ctx.Done():
			return
		case <-timer.C:
		}
		maintenanceRound(ctx, s, m)
		next = next.Add(sweep)
		if !next.After(time.Now()) {
			next = time.Now().Add(sweep)
		}
		timer.Reset(time.Until(next))
	}
}

// maintenanceRound retires a Cloud past its lifetime cap before it considers a checkpoint.
func maintenanceRound(ctx context.Context, s Shard, m *machine) {
	c := s.Cloud()
	tuning := s.CloudServices().Tuning
	c.mu.Lock()
	eligible := !c.stopped && m.phase == webapiproto.CloudStateRunning && m.transition == nil && m.retirement == nil
	started, checkpoint := m.started, m.checkpoint
	c.mu.Unlock()
	if !eligible || m.gate.State().Maintenance > 0 {
		return
	}
	now := time.Now()
	if now.Sub(started) >= tuning.LifetimeCap {
		stopAtCap(ctx, s, m)
		return
	}
	if now.Sub(checkpoint) < tuning.CheckpointInterval {
		return
	}
	lease := m.gate.TryEnter(gates.Maintenance)
	if lease == nil {
		return
	}
	c.mu.Lock()
	if c.stopped || m.phase != webapiproto.CloudStateRunning || m.transition != nil || m.retirement != nil {
		c.mu.Unlock()
		lease.Release()
		return
	}
	c.workers.Add(1)
	c.mu.Unlock()
	go func() {
		defer c.workers.Done()
		defer lease.Release()
		link := s.Devices().Link(m.device.ID)
		if link != nil {
			link.PauseLiveness()
			defer link.ResumeLiveness()
		}
		_, err := Call(
			context.WithoutCancel(ctx),
			s.CloudServices().Machines,
			machineproto.CheckpointParams{DeviceID: string(m.device.ID)},
		)
		if err != nil {
			slog.Warn("the Cloud's checkpoint failed", "device", m.device.ID, "error", err)
			return
		}
		c.mu.Lock()
		if m.phase == webapiproto.CloudStateRunning {
			m.checkpoint = now
		}
		c.mu.Unlock()
	}()
}

// stopAtCap fences new admissions before disconnecting unattended jobs, then
// drains their leases. The fence and the eventual reservation have one owner.
func stopAtCap(ctx context.Context, s Shard, m *machine) {
	uses, err := cloudUses(ctx, s)
	if err != nil {
		slog.Warn("the Cloud's lifetime cap could not read its conversations", "error", err)
		return
	}
	for _, use := range uses {
		if s.Attended(use.id) {
			return
		}
	}
	c := s.Cloud()
	c.mu.Lock()
	if c.stopped || m.phase != webapiproto.CloudStateRunning || m.transition != nil || m.retirement != nil {
		c.mu.Unlock()
		return
	}
	retirement := &transition{done: make(chan struct{})}
	m.retirement = retirement
	c.workers.Add(1)
	c.mu.Unlock()
	go func() {
		defer c.workers.Done()
		defer func() {
			c.mu.Lock()
			m.retirement = nil
			c.mu.Unlock()
			close(retirement.done)
		}()
		retireAtCap(ctx, s, m)
	}()
}

type cloudIdle struct {
	s Shard
	m *machine
}

// Check combines Cloud demand, jobs, transitions and all conversation roles.
func (p *cloudIdle) Check(ctx context.Context) (idlewatch.Activity, error) {
	activity := idlewatch.Of(p.m.gate.State())
	c := p.s.Cloud()
	c.mu.Lock()
	activity.Busy = activity.Busy || p.m.phase != webapiproto.CloudStateRunning || p.m.transition != nil ||
		p.m.retirement != nil
	c.mu.Unlock()
	if link := p.s.Devices().Link(p.m.device.ID); link != nil {
		activity.Busy = activity.Busy || link.RunningJobs() > 0
	}
	uses, err := cloudUses(ctx, p.s)
	if err != nil {
		return activity, err
	}
	for _, use := range uses {
		activity = activity.And(p.s.Activity(use.id))
	}
	return activity, nil
}

// Reserve holds the machine and all required conversations, or releases everything.
func (p *cloudIdle) Reserve(ctx context.Context) (idlewatch.Retirement, bool, error) {
	// Hold conversations first. Acquiring then releasing the machine gate on a
	// failed conversation hold would wake our own Changed subscription forever.
	uses, err := cloudUses(ctx, p.s)
	if err != nil {
		return nil, false, err
	}
	held, ok := holdIdle(p.s, uses)
	if !ok {
		return nil, false, nil
	}
	reserved := p.m.gate.TryReserve()
	if reserved == nil {
		releaseHolds(held)
		return nil, false, nil
	}
	return &idleRetirement{policy: p, reserved: reserved, held: held}, true, nil
}

// Changed subscribes before the idle watch tries reservation again.
func (p *cloudIdle) Changed() <-chan struct{} {
	return p.m.gate.State().Changed()
}

type idleRetirement struct {
	policy   *cloudIdle
	reserved *gates.Reservation
	held     []ConversationHold
}

// Retire commits the reserved idle stop.
func (r *idleRetirement) Retire(ctx context.Context) error {
	return hibernate(ctx, r.policy.s, r.policy.m)
}

// Release lets every conversation and the machine resume admission.
func (r *idleRetirement) Release() {
	releaseHolds(r.held)
	r.reserved.Release()
}

// retireAtCap drains jobs and rechecks attendance under conversation holds.
func retireAtCap(ctx context.Context, s Shard, m *machine) {
	reserved := m.gate.TryReserve()
	if reserved == nil {
		s.Devices().Disconnect(m.device.ID, "Cloud reached its lifetime cap")
		wait, cancel := context.WithTimeout(context.WithoutCancel(ctx), s.CloudServices().Tuning.ResetHold)
		var err error
		reserved, err = m.gate.Reserve(wait)
		cancel()
		if err != nil {
			slog.Warn("the Cloud's jobs did not end at its lifetime cap", "device", m.device.ID)
			return
		}
	}
	defer reserved.Release()
	// The read and reservation can race conversation work; re-read and check
	// attendance before committing retirement under held conversation gates.
	uses, err := cloudUses(context.WithoutCancel(ctx), s)
	if err != nil {
		return
	}
	for _, use := range uses {
		if s.Attended(use.id) {
			return
		}
	}
	held, ok := holdIdle(s, uses)
	if !ok {
		return
	}
	defer releaseHolds(held)
	if err := hibernate(context.WithoutCancel(ctx), s, m); err != nil {
		slog.Warn("the Cloud was not saved at its lifetime cap", "error", err)
	}
}
