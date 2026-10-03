package server

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
)

// Tree is a conversation's live nodes, connection attachments and supervisor.
// It belongs to its Server and exposes synchronized operations, not mutable state.
type Tree[H host.Host] struct {
	server          *Server[H]
	id              core.NodeID
	root            *Node[H]
	store           store.TreeStore
	admission       *gates.Activity
	actionAdmission gates.Serial
	profiles        []core.Profile
	toolset         string
	starts          gates.KeyedSerial[core.NodeID]
	children        map[core.NodeID]*child[H]
	changing        map[core.NodeID]int
	attachments     map[*Connection[H]]struct{}
	attachmentEpoch uint64
	changed         chan struct{}
	disposing       bool
	ctx             context.Context
	cancel          context.CancelFunc
	workers         sync.WaitGroup
	lifecycle       sync.WaitGroup
	subscription    *session.Subscription
	// Frame publication takes this mutex before the server mutex. Session
	// callbacks run outside session state locks, so snapshots cannot invert it.
	frames  sync.Mutex
	running map[core.CommandID]bool
	waiting []waitingOutput
	sent    time.Time
}

// Toolset returns the revision the tree opened with, for product comparison.
func (t *Tree[H]) Toolset() string { return t.toolset }

// Root returns the root node.
func (t *Tree[H]) Root() *Node[H] { return t.root }

// Node returns the live node, root or child at any depth, or nil.
func (t *Tree[H]) Node(id core.NodeID) *Node[H] {
	t.server.mu.Lock()
	defer t.server.mu.Unlock()
	if id == t.id {
		return t.root
	}
	if child := t.children[id]; child != nil {
		return child.node
	}
	return nil
}

// Admission returns the tree's admission. Every node action holds a lease;
// a target switch or archive reserves the idle tree through the same gate.
func (t *Tree[H]) Admission() *gates.Activity { return t.admission }

// Interrupt reserves admission, stops the root's running action, aborts all
// live children and ends the root's shells for a Host transition. The caller
// releases the reservation before queued actions may proceed.
func (t *Tree[H]) Interrupt(ctx context.Context) (*gates.Reservation, error) {
	permit, err := t.actionAdmission.Acquire(ctx)
	if err != nil {
		return nil, err
	}
	defer permit.Release()
	type answer struct {
		reservation *gates.Reservation
		err         error
	}
	result := make(chan answer, 1)
	reservationCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	go func() { r, err := t.admission.Reserve(reservationCtx); result <- answer{r, err} }()
	// New action admission remains gated until the reservation is acquired.
	err = t.root.session.StopRunning(ctx)
	err = errors.Join(err, t.abortChildren(ctx, t.id), t.root.runtime.access.Environments.EndAll(ctx))
	if err != nil {
		cancel()
	}
	got := <-result
	if err != nil || got.err != nil {
		if got.reservation != nil {
			got.reservation.Release()
		}
		return nil, errors.Join(err, got.err)
	}
	return got.reservation, nil
}

// IsAttached reports whether any connection is attached.
func (t *Tree[H]) IsAttached() bool {
	t.server.mu.Lock()
	defer t.server.mu.Unlock()
	return len(t.attachments) != 0
}

// IsQuiescent reports that no child or command is live and the root has no
// running or waiting action or scheduled wakeup.
func (t *Tree[H]) IsQuiescent() bool {
	t.server.mu.Lock()
	empty := len(t.children)+len(t.changing)+len(t.running) == 0
	t.server.mu.Unlock()
	status := t.root.session.Status()
	return empty && status.Settle == session.Settled && !status.Wakeups
}

// openTree assembles and publishes a single conversation under its opening gate.
func (s *Server[H]) openTree(
	ctx context.Context,
	root core.NodeID,
	cwd string,
) (*Tree[H], *session.Continuation, error) {
	toolset, err := s.deps.Toolsets.Current(ctx)
	if err != nil {
		return nil, nil, fmt.Errorf("the commands the conversation opens with cannot be read: %w", err)
	}
	if err := checkProfiles(toolset.Profiles); err != nil {
		return nil, nil, err
	}
	model, err := s.deps.Providers.Selection(ctx, root)
	if err != nil {
		return nil, nil, err
	}
	runtime, err := s.deps.Providers.Runtime(ctx, root, model)
	if err != nil {
		return nil, nil, err
	}
	lifetime, cancel := context.WithCancel(context.Background())
	t := &Tree[H]{
		server:      s,
		id:          root,
		store:       s.deps.Stores(root),
		admission:   gates.NewActivity(nil),
		profiles:    toolset.Profiles,
		toolset:     toolset.Revision,
		children:    map[core.NodeID]*child[H]{},
		changing:    map[core.NodeID]int{},
		attachments: map[*Connection[H]]struct{}{},
		changed:     make(chan struct{}),
		ctx:         lifetime,
		cancel:      cancel,
		running:     map[core.CommandID]bool{},
	}
	node, continuation, err := t.assemble(ctx, assembly{
		record:       store.RootRecord(root, s.deps.Clock.Now()),
		cwd:          cwd,
		model:        model,
		runtime:      runtime,
		instructions: s.deps.Instructions,
		preamble:     nil,
		inherited:    toolset.Commands,
		first:        nil,
	})
	if err != nil {
		cancel()
		return nil, nil, err
	}
	t.root = node
	if continuation != nil {
		if err := node.session.UpdateModel(session.ModelSwitch{Model: model}); err != nil {
			cancel()
			return nil, nil, errors.Join(err, node.session.Dispose(context.WithoutCancel(ctx)))
		}
	}
	if err := s.publishTree(ctx, t, node, cancel); err != nil {
		return nil, nil, err
	}
	return t, continuation, nil
}

// bump announces a change after releasing the server's state lock.
func (t *Tree[H]) bump() {
	t.server.mu.Lock()
	old := t.changed
	t.changed = make(chan struct{})
	t.server.mu.Unlock()
	close(old)
}

func (t *Tree[H]) changes() <-chan struct{} {
	t.server.mu.Lock()
	defer t.server.mu.Unlock()
	return t.changed
}

// emit sends a session event in order to each attached socket.
func (t *Tree[H]) emit(frame framewire.ServerFrame) {
	t.frames.Lock()
	defer t.frames.Unlock()
	t.publish(frame)
}

func (t *Tree[H]) publish(frame framewire.ServerFrame) {
	connections := t.connections()
	for _, c := range connections {
		if !c.outbox.push(frame) {
			t.detach(c)
		}
	}
}

func (t *Tree[H]) detach(c *Connection[H]) {
	t.server.mu.Lock()
	_, attached := t.attachments[c]
	delete(t.attachments, c)
	observations := c.observations
	state := c.stateObservation
	c.stateObservation = nil
	c.observations = nil
	if len(t.attachments) == 0 {
		t.waiting = nil
	}
	t.server.mu.Unlock()
	for _, subscription := range observations {
		if subscription != state {
			subscription.Release()
		}
	}
	if state != nil {
		state.Release()
	}
	if attached {
		t.bump()
	}
}

// attach orders the initial snapshot before later event publication.
func (t *Tree[H]) attach(c *Connection[H]) {
	t.frames.Lock()
	defer t.frames.Unlock()
	t.server.mu.Lock()
	if c.detached.Load() {
		t.server.mu.Unlock()
		return
	}
	t.attachments[c] = struct{}{}
	t.attachmentEpoch++
	t.server.mu.Unlock()
	snapshot := t.observeConnection(c, t.root)
	frames := []framewire.ServerFrame{
		&framewire.OpenedFrame{},
		&framewire.TranscriptResetFrame{Blocks: snapshot.Transcript.Blocks, Version: snapshot.Transcript.Version},
		&framewire.PhaseFrame{Phase: snapshot.Phase},
		&framewire.QueueFrame{Queue: snapshot.Queue},
		&framewire.PendingSteersFrame{PendingSteers: snapshot.PendingSteers},
	}
	frames = append(frames, t.replay(c)...)
	frames = append(frames, t.liveCommands()...)
	for _, frame := range frames {
		if !c.outbox.push(frame) {
			t.detach(c)
			break
		}
	}
	t.bump()
}

func (t *Tree[H]) freshTranscripts(c *Connection[H]) []framewire.ServerFrame {
	snapshot := t.observeConnection(c, t.root).Transcript
	frames := []framewire.ServerFrame{
		&framewire.TranscriptResetFrame{Blocks: snapshot.Blocks, Version: snapshot.Version},
	}
	frames = append(frames, t.replay(c)...)
	return append(frames, t.liveCommands()...)
}

func (t *Tree[H]) liveCommands() []framewire.ServerFrame {
	frames := []framewire.ServerFrame{}
	for _, view := range t.root.liveViews() {
		frames = append(frames, tools.ShellOutput(nil, view))
	}
	for _, child := range t.descendants(t.id) {
		for _, view := range child.node.liveViews() {
			frames = append(frames, tools.ShellOutput(new(child.node.ID()), view))
		}
	}
	return frames
}

func (t *Tree[H]) continueRestored(ctx context.Context, continuation session.Continuation) error {
	err := t.root.continueFrom(ctx, continuation)
	t.restoreChildren(ctx, t.root)
	return err
}

func (t *Tree[H]) dispose(ctx context.Context) error {
	t.server.mu.Lock()
	t.disposing = true
	t.server.mu.Unlock()
	t.cancel()
	t.workers.Wait()
	t.lifecycle.Wait()
	var errs []error
	for _, child := range t.descendants(t.id) {
		errs = append(errs, child.node.session.Dispose(ctx))
		child.events.Release()
	}
	errs = append(errs, t.root.session.Dispose(ctx))
	t.subscription.Release()
	err := errors.Join(errs...)
	if err != nil {
		t.report(err)
	}
	t.emit(&framewire.ClosedFrame{})
	t.server.mu.Lock()
	connections := make([]*Connection[H], 0, len(t.attachments))
	for c := range t.attachments {
		connections = append(connections, c)
	}
	clear(t.attachments)
	clear(t.children)
	t.server.mu.Unlock()
	for _, c := range connections {
		t.detach(c)
	}
	t.bump()
	return err
}

func (t *Tree[H]) report(err error) { t.emit(&framewire.ErrorFrame{Message: err.Error()}) }

// monitor owns the detached-idle timer and working-state notifications.
func (t *Tree[H]) monitor() {
	timer := time.NewTimer(t.server.deps.Config.IdleTree)
	defer timer.Stop()
	timer.Stop()
	var timeout <-chan time.Time
	lastWorking, known := false, false
	idle := false
	var attachmentEpoch uint64
	for {
		changed := t.changes()
		status := t.root.session.Status()
		working := !t.IsQuiescent()
		if !known || working != lastWorking {
			known = true
			lastWorking = working
			t.server.deps.StatusChanged(t.id)
		}
		t.server.mu.Lock()
		epoch := t.attachmentEpoch
		attached := len(t.attachments) != 0
		t.server.mu.Unlock()
		nowIdle := !attached && !working
		if nowIdle != idle || epoch != attachmentEpoch {
			attachmentEpoch = epoch
			idle = nowIdle
			timer.Stop()
			timeout = nil
			if idle {
				timer.Reset(t.server.deps.Config.IdleTree)
				timeout = timer.C
			}
		}
		select {
		case <-t.ctx.Done():
			return
		case <-changed:
		case <-status.Changed():
		case <-timeout:
			done := t.server.evict(t, attachmentEpoch)
			select {
			case <-t.ctx.Done():
				return
			case <-done:
			}
			idle = false
		}
	}
}

func frameOf(event session.Event) framewire.ServerFrame {
	switch e := event.(type) {
	case *session.TranscriptChanged:
		return &framewire.TranscriptPatchFrame{Patches: e.Patches, Revision: e.Revision}
	case *session.PhaseChanged:
		return &framewire.PhaseFrame{Phase: e.Phase}
	case *session.QueueChanged:
		return &framewire.QueueFrame{Queue: e.Queue}
	case *session.PendingSteersChanged:
		return &framewire.PendingSteersFrame{PendingSteers: e.PendingSteers}
	case *session.RetryScheduled:
		return &framewire.RetryScheduledFrame{
			Attempt:     e.Attempt,
			DelayMs:     e.DelayMS,
			Code:        e.Code,
			Diagnostics: e.Diagnostics,
		}
	case *session.ErrorEvent:
		return &framewire.ErrorFrame{Message: e.Report.Message, Code: e.Report.Code, Diagnostics: e.Report.Diagnostics}
	case *session.ActionFailed, *session.EditCommitted:
		return nil
	}
	return nil
}

func (s *Server[H]) publishTree(ctx context.Context, t *Tree[H], node *Node[H], cancel context.CancelFunc) error {
	_, t.subscription = node.session.Observe(func(event session.Event) {
		if _, phase := event.(*session.PhaseChanged); phase {
			s.deps.StatusChanged(t.id)
		}
	})
	s.mu.Lock()
	if s.closing {
		s.mu.Unlock()
		cancel()
		t.subscription.Release()
		return errors.Join(session.ErrClosed, node.session.Dispose(context.WithoutCancel(ctx)))
	}
	s.trees[t.id] = t
	s.mu.Unlock()
	t.workers.Go(t.monitor)
	t.workers.Go(t.sendChanges)
	return nil
}

func checkProfiles(profiles []core.Profile) error {
	for _, profile := range profiles {
		if profile.Name == "default" {
			return errors.New(
				`subagent profile name "default" is reserved: omitting --profile already inherits the parent`,
			)
		}
	}
	return nil
}
