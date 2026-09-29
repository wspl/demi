//go:build linux

package machines

import (
	"context"
	"errors"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"uuid"

	"github.com/wspl/demi/go/machines/internal/fault"
	"github.com/wspl/demi/go/machines/internal/sandbox"
	"github.com/wspl/demi/go/machines/internal/storage"
	"github.com/wspl/demi/go/machinesproto"
	"github.com/wspl/demi/go/runnerproto"
)

// A kind says what a device's worker is asked to do.
type kind int

const (
	wake kind = iota
	// hibernate stops and saves; it is also what reconcile and shutdown do to
	// every device.
	hibernate
	checkpoint
	grow
	reset
	readRuntimeState
)

// A command is what a device's worker is asked to do: one of the operations that
// change a device, with its arguments, or a read of its runtime state.
type command struct {
	kind      kind
	boot      runnerproto.ManagedBoot
	volume    machinesproto.Volume
	bytes     uint64
	operation string
	base      machinesproto.BaseVersion
	// reply receives the outcome of an operation.
	reply chan<- error
	// state receives the answer of readRuntimeState.
	state chan<- machinesproto.RuntimeState
}

// A deviceWorker is a device's worker (docs/cloud/managed-hosts.md § Control and
// ownership): it owns the device's sandbox and runs the device's operations one at
// a time, in arrival order. The sandbox's exit is one more event between
// operations, never during one.
type deviceWorker struct {
	ctx    context.Context
	device machinesproto.DeviceID
	core   *Core
	// base is the base a device's first storage is made from.
	base    machinesproto.BaseVersion
	working *storage.WorkingPair
	runtime *sandbox.Sandbox
	deaths  chan<- machinesproto.DeviceID
	log     *slog.Logger
}

func newDeviceWorker(ctx context.Context, device machinesproto.DeviceID, core *Core, base machinesproto.BaseVersion, deaths chan<- machinesproto.DeviceID) *deviceWorker {
	return &deviceWorker{
		ctx:     ctx,
		device:  device,
		core:    core,
		base:    base,
		working: storage.NewWorkingPair(core.Config.Working(), device),
		deaths:  deaths,
		log:     slog.With("device", string(device)),
	}
}

// run serves the device until its manager closes the inbox. The sandbox's exit is
// not queued behind requests: stopping a sandbox waits for its waiter, which must
// never wait for room in this bounded queue.
func (w *deviceWorker) run(inbox <-chan command) {
	// A sandbox still held when the worker ends is left to the stop-post
	// recovery; only the wait for its exit goes.
	defer func() {
		if w.runtime != nil {
			w.runtime.Abandon()
		}
	}()
	for {
		var exited <-chan error
		if w.runtime != nil {
			exited = w.runtime.Exited()
		}
		select {
		case c, ok := <-inbox:
			if !ok {
				return
			}
			w.handle(c)
		case outcome := <-exited:
			w.runtime.ExitObserved()
			w.runtimeExited(outcome)
		}
	}
}

func (w *deviceWorker) handle(c command) {
	if c.kind == readRuntimeState {
		state := machinesproto.Stopped
		if w.runtime != nil {
			state = machinesproto.Running
		}
		// The channel holds one answer, so this never waits on a requester that
		// has gone.
		c.state <- state
		return
	}
	// As above: the operation completed even when no one waits for its outcome.
	c.reply <- w.operate(c)
}

func (w *deviceWorker) operate(c command) error {
	switch c.kind {
	case wake:
		return w.wake(c.boot)
	case hibernate:
		if err := w.stop(); err != nil {
			return err
		}
		return w.save()
	case checkpoint:
		return w.checkpoint()
	case grow:
		return w.grow(c.volume, c.bytes)
	case reset:
		return w.reset(c.operation, c.base)
	}
	return errors.New("not an operation of a device")
}

// runtimeExited handles a sandbox that exited without being asked to: it stops
// what remains, saves the working pair, and reports the death even when those
// fail.
func (w *deviceWorker) runtimeExited(outcome error) {
	if outcome != nil {
		w.log.Error("machines: Cloud runtime wait failed: " + outcome.Error())
	}
	recovered := w.stop()
	if recovered == nil {
		recovered = w.save()
	}
	if recovered != nil {
		w.log.Error("machines: Cloud recovery failed: " + Chain(recovered))
	}
	w.reportDeath()
}

func (w *deviceWorker) reportDeath() {
	// The server stops listening at shutdown, when no one is left to tell.
	select {
	case w.deaths <- w.device:
	case <-w.ctx.Done():
	}
}

// stop stops the sandbox, if any; a failed close keeps it, and its slot.
func (w *deviceWorker) stop() error {
	if w.runtime == nil {
		return nil
	}
	if err := w.runtime.Close(w.ctx, w.working); err != nil {
		return err
	}
	w.runtime = nil
	return nil
}

// save publishes the working pair, which no sandbox may be writing.
func (w *deviceWorker) save() error {
	if w.runtime != nil {
		return ErrActiveWriter
	}
	return w.working.Save(w.ctx, w.core.Tools, w.core.Store, w.device)
}

// A stage is a staging directory in the working directory, whose name starts with
// a dot so recovery removes it; the operation removes it on every path.
type stage struct {
	path string
}

func (w *deviceWorker) newStage(prefix string) (*stage, error) {
	path := filepath.Join(w.core.Config.Working(), "."+prefix+"-"+uuid.New().String())
	if err := os.Mkdir(path, 0o777); err != nil {
		return nil, err
	}
	return &stage{path: path}, nil
}

// remove removes the stage. One that stays behind has not replaced anything, and
// the next reconciliation removes it; the failure is logged.
func (s *stage) remove() {
	if err := storage.RemoveTree(s.path); err != nil {
		slog.Warn("machines: " + err.Error())
	}
}

// wake starts the device's sandbox on its newest storage: the working pair a
// crash left, saved first, else the committed generation, else a new one.
func (w *deviceWorker) wake(boot runnerproto.ManagedBoot) error {
	if w.runtime != nil {
		return nil
	}
	if err := w.save(); err != nil {
		return err
	}
	state, err := w.core.Store.Read(w.device)
	if err != nil {
		return err
	}
	if state == nil {
		initial, err := w.initialize(w.base)
		if err != nil {
			return err
		}
		state = &initial
	}
	stage, err := w.newStage("wake")
	if err != nil {
		return err
	}
	copied := w.stageWorking(*state, stage.path)
	stage.remove()
	if copied != nil {
		return copied
	}
	fault.Point("working-staged")
	lease, err := w.core.Slots.Take()
	if err != nil {
		return err
	}
	started := sandbox.New(&w.core.Host, lease)
	base := filepath.Join(w.core.Store.Bases(), string(state.BaseVersion), "rootfs")
	failure := started.Start(w.ctx, w.working, base, boot)
	if failure == nil {
		w.runtime = started
		return nil
	}
	if cleanup := started.Close(w.ctx, w.working); cleanup != nil {
		w.runtime = started
		return startAndCleanup(failure, cleanup)
	}
	return failure
}

// stageWorking copies the committed generation into stage and makes the stage the
// working pair.
func (w *deviceWorker) stageWorking(state machinesproto.ImageState, stage string) error {
	sources := w.core.Store.Images(w.device, state.Generation)
	copies := storage.ImagesIn(stage)
	for _, volume := range machinesproto.Volumes {
		if err := storage.CloneSparse(sources.Get(volume), copies.Get(volume)); err != nil {
			return err
		}
		if err := storage.Sync(copies.Get(volume)); err != nil {
			return err
		}
	}
	if err := storage.WriteState(filepath.Join(stage, "manifest.json"), state); err != nil {
		return err
	}
	if err := storage.Sync(stage); err != nil {
		return err
	}
	if err := os.Rename(stage, w.working.Directory()); err != nil {
		return err
	}
	return storage.Sync(filepath.Dir(w.working.Directory()))
}

// initialize makes a device's first storage: a home from the base's skeleton, an
// empty system, published as its first generation.
func (w *deviceWorker) initialize(base machinesproto.BaseVersion) (machinesproto.ImageState, error) {
	stage, err := w.newStage("initial")
	if err != nil {
		return machinesproto.ImageState{}, err
	}
	defer stage.remove()
	return w.makeFirstGeneration(base, stage.path)
}

func (w *deviceWorker) makeFirstGeneration(base machinesproto.BaseVersion, stage string) (machinesproto.ImageState, error) {
	skeleton := filepath.Join(w.core.Store.Bases(), string(base), "rootfs/etc/skel")
	root := filepath.Join(stage, "mkhome")
	// The home filesystem's root is the sandbox's /home.
	if err := os.Mkdir(root, 0o777); err != nil {
		return machinesproto.ImageState{}, err
	}
	if err := os.Chmod(root, 0o755); err != nil {
		return machinesproto.ImageState{}, err
	}
	if err := storage.CopySkeleton(skeleton, filepath.Join(root, "demi")); err != nil {
		return machinesproto.ImageState{}, err
	}
	config := w.core.Config
	images := storage.ImagesIn(stage)
	if err := storage.MakeHome(w.ctx, w.core.Tools, root, images.Home, config.HomeBytes()); err != nil {
		return machinesproto.ImageState{}, err
	}
	if err := storage.MakeSystem(w.ctx, w.core.Tools, images.System, config.SystemBytes()); err != nil {
		return machinesproto.ImageState{}, err
	}
	state := machinesproto.ImageState{
		Generation:  storage.NewGeneration(),
		BaseVersion: base,
		SystemBytes: config.SystemBytes(),
		HomeBytes:   config.HomeBytes(),
	}
	for _, volume := range machinesproto.Volumes {
		if err := storage.Sync(images.Get(volume)); err != nil {
			return machinesproto.ImageState{}, err
		}
	}
	if err := w.core.Store.Publish(w.device, state, images); err != nil {
		return machinesproto.ImageState{}, err
	}
	return state, nil
}

// checkpoint publishes the running pair without stopping its processes: pause,
// the frozen window, resume, then publication of the copies. A thaw or resume that
// fails ends the sandbox and reports its death.
func (w *deviceWorker) checkpoint() error {
	if w.runtime == nil {
		return nil
	}
	state, err := w.working.Manifest()
	if err != nil {
		return err
	}
	if state == nil {
		return ErrNoWorkingManifest
	}
	stage, err := w.newStage("checkpoint")
	if err != nil {
		return err
	}
	defer stage.remove()
	return w.checkpointInto(*state, stage.path)
}

func (w *deviceWorker) checkpointInto(state machinesproto.ImageState, stage string) error {
	var captured error
	var cleanup []error
	if err := w.runtime.Pause(w.ctx); err != nil {
		// Pausing failed: nothing is frozen, and the failure is reported once the
		// sandbox is resumed.
		captured = err
	} else {
		capture := w.runtime.Capture(w.working, stage)
		cleanup = capture.ThawErrors
		captured = capture.Copied
	}
	if err := w.runtime.Resume(w.ctx); err != nil {
		cleanup = append(cleanup, err)
	}
	if len(cleanup) > 0 {
		// A runtime that could not thaw or resume must not look running.
		if err := w.stop(); err != nil {
			cleanup = append(cleanup, err)
		}
		w.reportDeath()
		var parts []error
		if captured != nil {
			parts = append(parts, captured)
		}
		return checkpointRecovery(append(parts, cleanup...))
	}
	if captured != nil {
		return captured
	}
	saved := state
	saved.Generation = storage.NewGeneration()
	return w.core.Store.Publish(w.device, saved, storage.ImagesIn(stage))
}

// grow grows a running volume, never shrinking it, and records the capacity its
// filesystem reports.
func (w *deviceWorker) grow(volume machinesproto.Volume, bytes uint64) error {
	if w.runtime == nil {
		return ErrNotRunning
	}
	state, err := w.working.Manifest()
	if err != nil {
		return err
	}
	if state == nil {
		return ErrNoWorkingManifest
	}
	if bytes <= state.Bytes(volume) {
		return nil
	}
	capacity, err := w.runtime.Grow(w.ctx, w.working, volume, bytes)
	if err != nil {
		return err
	}
	return w.working.WriteManifest(state.WithBytes(volume, capacity))
}

// reset publishes a fresh system on base with the saved home, once per operation;
// it does not boot.
func (w *deviceWorker) reset(operation string, base machinesproto.BaseVersion) error {
	manifest := filepath.Join(w.core.Store.Bases(), string(base), "manifest.json")
	if _, err := os.Stat(manifest); errors.Is(err, fs.ErrNotExist) {
		return &MissingBaseError{Base: base}
	} else if err != nil {
		return err
	}
	if err := w.stop(); err != nil {
		return err
	}
	if err := w.save(); err != nil {
		return err
	}
	state, err := w.core.Store.Read(w.device)
	if err != nil {
		return err
	}
	if state == nil {
		initial, err := w.initialize(base)
		if err != nil {
			return err
		}
		state = &initial
	}
	if state.ResetID != nil && *state.ResetID == operation {
		return nil
	}
	stage, err := w.newStage("reset")
	if err != nil {
		return err
	}
	defer stage.remove()
	return w.publishReset(*state, operation, base, stage.path)
}

func (w *deviceWorker) publishReset(state machinesproto.ImageState, operation string, base machinesproto.BaseVersion, stage string) error {
	system := storage.ImagesIn(stage).System
	systemBytes := w.core.Config.SystemBytes()
	if err := storage.MakeSystem(w.ctx, w.core.Tools, system, systemBytes); err != nil {
		return err
	}
	fault.Point("reset-staged")
	// The home enters the new generation as a link to the saved image.
	home := w.core.Store.Images(w.device, state.Generation).Home
	reset := machinesproto.ImageState{
		Generation:  storage.NewGeneration(),
		BaseVersion: base,
		ResetID:     &operation,
		SystemBytes: systemBytes,
		HomeBytes:   state.HomeBytes,
	}
	if err := storage.Sync(system); err != nil {
		return err
	}
	return w.core.Store.Publish(w.device, reset, storage.ImagePair[string]{System: system, Home: home})
}
