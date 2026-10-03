//go:build linux

package machines

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"

	"github.com/wspl/demi/internal/machines/sandbox"
	"github.com/wspl/demi/internal/machines/storage"
	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
)

type deviceRequest struct {
	ctx       context.Context
	operation machinewire.MachineCall
	reply     chan deviceReply
}
type deviceReply struct {
	value json.RawMessage
	err   error
}
type bootExit struct {
	id  sandbox.ID
	err error
}
type deviceWorker struct {
	manager    *Manager
	id         machinewire.DeviceID
	working    workingImages
	runtime    *sandbox.Sandbox
	queue      chan deviceRequest
	exit       chan bootExit
	cancelWait context.CancelFunc
	waitDone   chan struct{}
}

func newDeviceWorker(m *Manager, id machinewire.DeviceID) *deviceWorker {
	return &deviceWorker{
		manager: m,
		id:      id,
		working: workingImages{storage.NewWorkingPair(m.core.Config.Working(), id)},
		queue:   make(chan deviceRequest, 64),
	}
}

func (w *deviceWorker) call(ctx context.Context, operation machinewire.MachineCall) (json.RawMessage, error) {
	request := deviceRequest{ctx: ctx, operation: operation, reply: make(chan deviceReply, 1)}
	w.queue <- request
	answer := <-request.reply
	return answer.value, answer.err
}

func (w *deviceWorker) run(ctx context.Context) {
	defer w.stopWait(context.WithoutCancel(ctx))
	for {
		select {
		case request, ok := <-w.queue:
			if !ok {
				if w.runtime != nil {
					if err := w.runtime.StopWaiting(context.Background()); err != nil {
						slog.Error(err.Error())
					}
				}
				return
			}
			value, err := w.operate(request.ctx, request.operation)
			request.reply <- deviceReply{value, err}
		case exit := <-w.exit:
			w.exit = nil
			if w.runtime == nil || w.runtime.ID() != exit.id {
				continue
			}
			if exit.err != nil {
				slog.Error("machines: Cloud runtime wait failed: " + exit.err.Error())
			}
			ctx := context.Background()
			err := w.stop(ctx)
			if err == nil {
				err = w.save(ctx)
			}
			if err != nil {
				slog.Error("machines: Cloud recovery failed: " + err.Error())
			}
			w.reportDeath(ctx)
		}
	}
}

func (w *deviceWorker) stopWait(_ context.Context) {
	if w.cancelWait != nil {
		w.cancelWait()
		<-w.waitDone
		w.cancelWait = nil
		w.exit = nil
	}
}

func (w *deviceWorker) startWait(ctx context.Context) {
	w.stopWait(ctx)
	ctx, cancel := context.WithCancel(context.WithoutCancel(ctx))
	w.cancelWait = cancel
	w.waitDone = make(chan struct{})
	w.exit = make(chan bootExit, 1)
	runtime, exit, done := w.runtime, w.exit, w.waitDone
	go func() {
		defer close(done)
		err := runtime.Exited(ctx)
		exit <- bootExit{runtime.ID(), err}
	}()
}

func (w *deviceWorker) reportDeath(_ context.Context) {
	select {
	case w.manager.deaths <- w.id:
	case <-w.manager.stopping:
	}
}

func (w *deviceWorker) operate(ctx context.Context, operation machinewire.MachineCall) (json.RawMessage, error) {
	var err error
	switch call := operation.(type) {
	case *machinewire.RuntimeStateCall:
		state := machinewire.RuntimeStateStopped
		if w.runtime != nil {
			state = machinewire.RuntimeStateRunning
		}
		return state.MarshalJSON()
	case *machinewire.Wake:
		err = w.wake(ctx, call.Params.Boot)
	case *machinewire.Hibernate:
		err = w.stop(ctx)
		if err == nil {
			err = w.save(ctx)
		}
	case *machinewire.Checkpoint:
		err = w.checkpoint(ctx)
	case *machinewire.GrowVolume:
		err = w.grow(ctx, call.Params.Volume, call.Params.Bytes)
	case *machinewire.Reset:
		err = w.reset(ctx, call.Params.OperationID, machinewire.BaseVersion(call.Params.BaseVersion))
	case *machinewire.Reconcile, *machinewire.CurrentBaseVersion, *machinewire.ImageState:
		err = errors.New("the device's worker stopped")
	}
	return json.RawMessage("null"), err
}

func (w *deviceWorker) stop(ctx context.Context) error {
	if w.runtime == nil {
		return nil
	}
	if err := w.runtime.Close(ctx, w.working); err != nil {
		return err
	}
	w.stopWait(ctx)
	w.runtime = nil
	return nil
}

func (w *deviceWorker) save(ctx context.Context) error {
	if w.runtime != nil {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return errors.New("Cannot publish working disks with an active writer")
	}
	return w.working.Save(ctx, w.manager.core.Tools, w.manager.core.Store, w.id)
}

// stage creates a private scratch directory for one Cloud storage operation.
func (w *deviceWorker) stage(ctx context.Context, name string) (string, func(), error) {
	id, err := storage.NewGeneration()
	if err != nil {
		return "", nil, err
	}
	path := filepath.Join(w.manager.core.Config.Working(), "."+name+"-"+string(id))
	if err = storage.CreatePrivate(ctx, path); err != nil {
		return "", nil, err
	}
	return path, func() {
		if err := storage.RemoveTree(context.WithoutCancel(ctx), path); err != nil {
			slog.Warn("machines: " + err.Error())
		}
	}, nil
}

func (w *deviceWorker) wake(ctx context.Context, boot runnerwire.ManagedBoot) error {
	if w.runtime != nil {
		return nil
	}
	if err := w.save(ctx); err != nil {
		return err
	}
	core := w.manager.core
	state, err := core.Store.Read(ctx, w.id)
	if err != nil {
		return err
	}
	if state == nil {
		state, err = w.initialize(ctx, w.manager.base)
		if err != nil {
			return err
		}
	}
	stage, remove, err := w.stage(ctx, "wake")
	if err != nil {
		return err
	}
	defer remove()
	if err = w.stageWorking(ctx, *state, stage); err != nil {
		return err
	}
	system.FaultPoint("working-staged")
	lease, err := core.Slots.Take()
	if err != nil {
		return err
	}
	slot := lease.Slot()
	runtime, err := sandbox.New(
		core.sandboxConfig(),
		core.dependencies(),
		sandbox.Slot{Index: slot.Index, Namespace: slot.Namespace()},
		lease,
	)
	if err != nil {
		lease.Release()
		return err
	}
	err = runtime.Start(ctx, w.working, filepath.Join(core.Store.Bases(), string(state.BaseVersion), "rootfs"), boot)
	if err == nil {
		w.runtime = runtime
		w.startWait(ctx)
		return nil
	}
	if cleanup := runtime.Close(ctx, w.working); cleanup != nil {
		w.runtime = runtime
		w.startWait(ctx)
		return &OperationError{
			Message: "Cloud start and cleanup failed; working storage retained",
			Cause:   errors.Join(err, cleanup),
		}
	}
	return err
}

func (w *deviceWorker) stageWorking(ctx context.Context, state machinewire.MachineImageState, stage string) error {
	source := w.manager.core.Store.Images(w.id, state.Generation)
	copies := storage.ImagesInDirectory(stage)
	for _, v := range []machinewire.Volume{machinewire.VolumeSystem, machinewire.VolumeHome} {
		if err := storage.CloneSparse(ctx, source.ForVolume(v), copies.ForVolume(v)); err != nil {
			return err
		}
		if err := storage.Sync(ctx, copies.ForVolume(v)); err != nil {
			return err
		}
	}
	if err := storage.WriteJSON(ctx, filepath.Join(stage, "manifest.json"), state); err != nil {
		return err
	}
	if err := storage.Sync(ctx, stage); err != nil {
		return err
	}
	if err := os.Rename(stage, w.working.Directory()); err != nil {
		return err
	}
	return storage.Sync(ctx, filepath.Dir(w.working.Directory()))
}

func (w *deviceWorker) initialize(
	ctx context.Context,
	base machinewire.BaseVersion,
) (*machinewire.MachineImageState, error) {
	stage, remove, err := w.stage(ctx, "initial")
	if err != nil {
		return nil, err
	}
	defer remove()
	core := w.manager.core
	root := filepath.Join(stage, "mkhome")
	if err = os.Mkdir(root, 0o755); err != nil {
		return nil, err
	}
	if err = os.Chmod(root, 0o755); err != nil {
		return nil, err
	}
	if err = storage.CopySkeleton(
		ctx,
		filepath.Join(core.Store.Bases(), string(base), "rootfs/etc/skel"),
		filepath.Join(root, "demi"),
	); err != nil {
		return nil, err
	}
	images := storage.ImagesInDirectory(stage)
	if err = storage.MakeHome(ctx, core.Tools, root, images.Home, core.Config.HomeBytes()); err != nil {
		return nil, err
	}
	if err = storage.MakeSystem(ctx, core.Tools, images.System, core.Config.SystemBytes()); err != nil {
		return nil, err
	}
	generation, err := storage.NewGeneration()
	if err != nil {
		return nil, err
	}
	state := machinewire.MachineImageState{
		Generation:  generation,
		BaseVersion: base,
		SystemBytes: core.Config.SystemBytes(),
		HomeBytes:   core.Config.HomeBytes(),
	}
	for _, path := range []string{images.System, images.Home} {
		if err = storage.Sync(ctx, path); err != nil {
			return nil, err
		}
	}
	if err = core.Store.Publish(ctx, w.id, state, images); err != nil {
		return nil, err
	}
	return &state, nil
}

func (w *deviceWorker) checkpoint(ctx context.Context) error {
	if w.runtime == nil {
		return nil
	}
	state, err := w.working.Manifest(ctx)
	if err != nil {
		return err
	}
	if state == nil {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return errors.New("Cloud working manifest is missing")
	}
	stage, remove, err := w.stage(ctx, "checkpoint")
	if err != nil {
		return err
	}
	defer remove()
	copies := imagePaths{storage.ImagesInDirectory(stage)}
	captured := w.runtime.Pause(ctx)
	var cleanup []error
	if captured == nil {
		capture := w.runtime.Capture(ctx, w.working, copies)
		captured = capture.Copied
		cleanup = capture.ThawErrors
	}
	if err = w.runtime.Resume(ctx); err != nil {
		cleanup = append(cleanup, err)
	}
	if len(cleanup) > 0 {
		if err = w.stop(ctx); err != nil {
			cleanup = append(cleanup, err)
		}
		w.reportDeath(ctx)
		return &OperationError{
			Message: "Cloud checkpoint recovery failed",
			Cause:   errors.Join(append([]error{captured}, cleanup...)...),
		}
	}
	if captured != nil {
		return captured
	}
	state.Generation, err = storage.NewGeneration()
	if err != nil {
		return err
	}
	return w.manager.core.Store.Publish(ctx, w.id, *state, copies.ImagePair)
}

func (w *deviceWorker) grow(ctx context.Context, volume machinewire.Volume, bytes uint64) error {
	if w.runtime == nil {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return errors.New("Cloud is not running")
	}
	state, err := w.working.Manifest(ctx)
	if err != nil {
		return err
	}
	if state == nil {
		//nolint:staticcheck // User-visible text is copied verbatim from Rust.
		return errors.New("Cloud working manifest is missing")
	}
	current := state.HomeBytes
	if volume == machinewire.VolumeSystem {
		current = state.SystemBytes
	}
	if bytes <= current {
		return nil
	}
	capacity, err := w.runtime.Grow(ctx, w.working, volume, bytes)
	if err != nil {
		return err
	}
	if volume == machinewire.VolumeSystem {
		state.SystemBytes = capacity
	} else {
		state.HomeBytes = capacity
	}
	return w.working.WriteManifest(ctx, *state)
}

func (w *deviceWorker) reset(ctx context.Context, operation string, base machinewire.BaseVersion) error {
	core := w.manager.core
	if _, err := os.Stat(filepath.Join(core.Store.Bases(), string(base), "manifest.json")); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			//nolint:staticcheck // User-visible text is copied verbatim from Rust.
			return fmt.Errorf("Cloud base %s is not imported", base)
		}
		return err
	}
	if err := w.stop(ctx); err != nil {
		return err
	}
	if err := w.save(ctx); err != nil {
		return err
	}
	state, err := core.Store.Read(ctx, w.id)
	if err != nil {
		return err
	}
	if state == nil {
		state, err = w.initialize(ctx, base)
		if err != nil {
			return err
		}
	}
	if state.ResetID != nil && *state.ResetID == operation {
		return nil
	}
	stage, remove, err := w.stage(ctx, "reset")
	if err != nil {
		return err
	}
	defer remove()
	systemImage := storage.ImagesInDirectory(stage).System
	if err = storage.MakeSystem(ctx, core.Tools, systemImage, core.Config.SystemBytes()); err != nil {
		return err
	}
	system.FaultPoint("reset-staged")
	home := core.Store.Images(w.id, state.Generation).Home
	state.Generation, err = storage.NewGeneration()
	if err != nil {
		return err
	}
	state.BaseVersion = base
	state.ResetID = &operation
	state.SystemBytes = core.Config.SystemBytes()
	if err = storage.Sync(ctx, systemImage); err != nil {
		return err
	}
	return core.Store.Publish(ctx, w.id, *state, storage.ImagePair[string]{System: systemImage, Home: home})
}
