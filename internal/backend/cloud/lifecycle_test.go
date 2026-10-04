package cloud

import (
	"context"
	"errors"
	"io"
	"reflect"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/idlewatch"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// Lifecycle scenarios use virtual time and in-memory manager/storage boundaries.
// Each runs in well under one second; none needs a real manager or model.
func TestConcurrentWakeOwnsTransitionAfterCallerLeaves(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		wake := make(chan struct{})
		f.hook = func(call machinewire.Call) (string, error) {
			if call.Name() == "wake" {
				<-wake
			}
			return "", nil
		}
		ctx, cancel := context.WithCancel(t.Context())
		left := make(chan error, 1)
		go func() {
			a, err := Access(ctx, f)
			if a != nil {
				a.Release()
			}
			left <- err
		}()
		f.waitCalls(t.Context(), "wake", 1)
		cancel()
		if err := <-left; !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
		const callers = 8
		results := make(chan *MachineAccess, callers)
		var joined sync.WaitGroup
		for range callers {
			joined.Go(func() {
				a, err := Access(t.Context(), f)
				if err != nil {
					t.Error(err)
				}
				results <- a
			})
		}
		synctest.Wait()
		if f.count("wake") != 1 {
			t.Fatal("concurrent callers booted twice")
		}
		close(wake)
		joined.Wait()
		for range callers {
			a := <-results
			if a == nil {
				t.Fatal("missing admission")
			}
			if a.Home != "/home/demi" {
				t.Fatal(a.Home)
			}
			a.Release()
		}
		if f.phase() != webapi.CloudStateRunning {
			t.Fatal(f.phase())
		}
		if len(f.records.tokens) != 1 {
			t.Fatal("rotated token more than once")
		}
	})
}

func TestBootTimeoutAndShutdownSaveAndReleaseCapacity(t *testing.T) {
	for _, shutdown := range []bool{false, true} {
		t.Run(map[bool]string{false: "timeout", true: "shutdown"}[shutdown], func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				f := newCloudFixture(t)
				f.connect = false
				f.services.Capacity = NewCapacity(1)
				result := make(chan error, 1)
				start := time.Now()
				go func() {
					_, err := Access(t.Context(), f)
					result <- err
				}()
				f.waitCalls(t.Context(), "wake", 1)
				synctest.Wait()
				if shutdown {
					f.cloud.Stop()
				}
				err := <-result
				if shutdown {
					requireCloudError(t, err, Closed)
				} else {
					requireCloudError(t, err, Failed)
					if time.Since(start) != 3*time.Second {
						t.Fatal(time.Since(start))
					}
				}
				if f.phase() != webapi.CloudStateOff || f.count("hibernate") != 1 {
					t.Fatalf("failed boot not saved: %s", f.phase())
				}
				permit := f.services.Capacity.TryTake()
				if permit == nil {
					t.Fatal("failed boot leaked capacity")
				}
				permit.Release()
			})
		})
	}
}

func TestRecoveryKeepsLiveTokenAndRebootsStoppedRuntime(t *testing.T) {
	for _, running := range []bool{true, false} {
		t.Run(map[bool]string{true: "live", false: "stopped"}[running], func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				f := newCloudFixture(t)
				a := f.access()
				a.Release()
				f.devices.Disconnect("cloud-device", "test transport loss")
				if !running {
					f.runtime = machinewire.RuntimeStateStopped
				}
				result := make(chan *MachineAccess, 1)
				go func() { result <- f.access() }()
				f.waitCalls(t.Context(), "runtime_state", 1)
				synctest.Wait()
				if running {
					f.connectRunner()
				}
				a = <-result
				a.Release()
				want := 1
				if !running {
					want = 2
				}
				if f.count("wake") != want || len(f.records.tokens) != want {
					t.Fatalf("wake/token count = %d/%d", f.count("wake"), len(f.records.tokens))
				}
				if f.count("reconcile") != 0 {
					t.Fatal("recovery reconciled every user's machines")
				}
			})
		})
	}
}

func TestRecoveryTimeoutPreservesRunningMachine(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		a := f.access()
		a.Release()
		f.devices.Disconnect("cloud-device", "test")
		start := time.Now()
		_, err := Access(t.Context(), f)
		requireCloudError(t, err, Failed)
		if err.Error() != "Cloud runner reconnect timeout" || time.Since(start) != 3*time.Second {
			t.Fatalf("recovery: %v after %s", err, time.Since(start))
		}
		if f.count("wake") != 1 || f.count("hibernate") != 0 || f.phase() != webapi.CloudStateRunning {
			t.Fatal("recovery destroyed live runtime")
		}
	})
}

func TestCapacityCrashLoopAndWindowExpiry(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.services.Capacity = NewCapacity(1)
		f.services.Tuning.CrashLoopDeaths = 2
		f.services.Tuning.CrashLoopWindow = 5 * time.Second
		permit := f.services.Capacity.TryTake()
		_, err := Access(t.Context(), f)
		requireCloudError(t, err, AtCapacity)
		if f.count("wake") != 0 {
			t.Fatal("full capacity still booted")
		}
		permit.Release()
		permit.Release()
		for range 2 {
			a := f.access()
			a.Release()
			if err := Died(t.Context(), f, "cloud-device"); err != nil {
				t.Fatal(err)
			}
		}
		_, err = Access(t.Context(), f)
		requireCloudError(t, err, CrashLoop)
		time.Sleep(5 * time.Second) // Advance the exact crash-loop window in virtual time.
		a := f.access()
		a.Release()
		if f.count("wake") != 3 {
			t.Fatal(f.count("wake"))
		}
	})
}

func TestIdleStopWaitsForDemandAndMaintenance(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.flush = true
		f.services.Tuning.CheckpointInterval = 3 * time.Second
		checkpoint := make(chan struct{})
		f.hook = func(call machinewire.Call) (string, error) {
			if call.Name() == "checkpoint" {
				<-checkpoint
			}
			return "", nil
		}
		a := f.access()
		start := time.Now()
		time.Sleep(12 * time.Second)
		if f.count("hibernate") != 0 {
			t.Fatal("idle stop interrupted admission")
		}
		a.Release()
		released := time.Now()
		time.Sleep(11 * time.Second)
		if f.count("hibernate") != 0 {
			t.Fatal("idle stop interrupted checkpoint")
		}
		if f.count("checkpoint") != 1 {
			t.Fatal("overlapping checkpoints")
		}
		close(checkpoint)
		f.waitCalls(t.Context(), "hibernate", 1)
		synctest.Wait()
		if time.Since(released) != 11*time.Second || time.Since(start) != 23*time.Second {
			t.Fatal("maintenance restarted idle clock")
		}
		if f.phase() != webapi.CloudStateOff {
			t.Fatal(f.phase())
		}
	})
}

func TestIdleUsesAttachedActivityAndReleasesPartialHolds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.flush = true
		f.records.uses = []database.CloudUseRecord{
			{ID: "target", OnCloud: true},
			{ID: "second", OnCloud: true},
			{ID: "attached", Attached: true},
		}
		f.activity["attached"] = idlewatch.Activity{Busy: true}
		a := f.access()
		a.Release()
		time.Sleep(12 * time.Second)
		if f.count("hibernate") != 0 {
			t.Fatal("attached conversation did not keep Cloud awake")
		}
		f.mu.Lock()
		f.activity["attached"] = idlewatch.Activity{LastDemandEnd: time.Now()}
		f.holdFailure = "second"
		f.mu.Unlock()
		time.Sleep(10 * time.Second)
		synctest.Wait()
		f.mu.Lock()
		holds := f.holds
		f.holdFailure = ""
		f.mu.Unlock()
		if holds != 0 || f.count("hibernate") != 0 {
			t.Fatal("partial idle reservation leaked or retired")
		}
		f.waitCalls(t.Context(), "hibernate", 1)
		synctest.Wait()
		for _, id := range f.heldIDs {
			if id == "attached" {
				t.Fatal("held attached-only conversation")
			}
		}
	})
}

func TestLifetimeCapDefersAttendanceAndDrainsLeases(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.services.Tuning.LifetimeCap = 3 * time.Second
		f.records.uses = []database.CloudUseRecord{{ID: "conversation", Attached: true}}
		f.attended["conversation"] = true
		a := f.access()
		time.Sleep(4 * time.Second)
		if f.count("hibernate") != 0 || !f.devices.Online("cloud-device") {
			t.Fatal("cap interrupted attended work")
		}
		f.mu.Lock()
		f.attended["conversation"] = false
		f.mu.Unlock()
		time.Sleep(time.Second)
		synctest.Wait()
		if f.devices.Online("cloud-device") {
			t.Fatal("unattended runner was not disconnected")
		}
		if lease, err := a.admission.PerOperation(); err == nil {
			lease.Release()
			t.Fatal("work slipped into draining cap")
		}
		a.Release()
		f.waitCalls(t.Context(), "hibernate", 1)
		synctest.Wait()
		if f.phase() != webapi.CloudStateOff {
			t.Fatal(f.phase())
		}
	})
}

func TestResetIdempotencyAndWaitingAdmission(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.flush = true
		f.services.Capacity = NewCapacity(1)
		f.records.uses = []database.CloudUseRecord{{ID: "target", OnCloud: true}, {ID: "attached", Attached: true}}
		a := f.access()
		a.Release()
		rebuild := make(chan struct{})
		f.hook = func(call machinewire.Call) (string, error) {
			if call.Name() == "reset" {
				<-rebuild
			}
			return "", nil
		}
		op, err := Reset(t.Context(), f, "reset-one")
		if err != nil {
			t.Fatal(err)
		}
		f.waitCalls(t.Context(), "reset", 1)
		same, err := Reset(t.Context(), f, op.ID)
		if err != nil || same.ID != op.ID {
			t.Fatalf("same reset: %+v %v", same, err)
		}
		_, err = Reset(t.Context(), f, "reset-two")
		requireCloudError(t, err, Resetting)
		pending := make(chan *MachineAccess, 1)
		go func() { pending <- f.access() }()
		synctest.Wait()
		select {
		case <-pending:
			t.Fatal("admission escaped reset")
		default:
		}
		close(rebuild)
		a = <-pending
		a.Release()
		ready, err := Reset(t.Context(), f, op.ID)
		if err != nil || ready.Phase != webapi.ResetPhaseReady {
			t.Fatalf("ready: %+v %v", ready, err)
		}
		if f.count("reset") != 1 || f.count("wake") != 2 || f.count("current_base_version") != 1 {
			t.Fatal("reset not idempotent")
		}
		want := []webapi.ResetPhase{
			webapi.ResetPhaseStopping,
			webapi.ResetPhaseSaving,
			webapi.ResetPhaseRebuilding,
			webapi.ResetPhaseBooting,
			webapi.ResetPhaseReady,
		}
		if !reflect.DeepEqual(f.records.phases, want) {
			t.Fatalf("phases: %v", f.records.phases)
		}
		if !reflect.DeepEqual(f.heldIDs, []webapi.ConversationID{"target"}) ||
			!reflect.DeepEqual(f.resetFiles, []bool{true}) {
			t.Fatal("reset held wrong conversations")
		}
		if p := f.services.Capacity.TryTake(); p != nil {
			p.Release()
			t.Fatal("running reset lost capacity")
		}
	})
}

func TestFailedResetRetryUsesPinnedBaseAndReleasesHolds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.flush = true
		f.records.uses = []database.CloudUseRecord{{ID: "target", OnCloud: true}}
		fail := true
		f.hook = func(call machinewire.Call) (string, error) {
			if call.Name() == "reset" && fail {
				return "", errors.New("disk refused")
			}
			return "", nil
		}
		_, err := Reset(t.Context(), f, "retry")
		if err != nil {
			t.Fatal(err)
		}
		f.cloud.mu.Lock()
		task := f.cloud.machine.reset
		f.cloud.mu.Unlock()
		if task == nil {
			t.Fatal("missing reset")
		}
		err = task.wait(t.Context())
		requireCloudError(t, err, Failed)
		if f.phase() != webapi.CloudStateOff || f.holds != 0 {
			t.Fatal("failed reset leaked phase or holds")
		}
		fail = false
		_, err = Reset(t.Context(), f, "retry")
		if err != nil {
			t.Fatal(err)
		}
		a := f.access()
		a.Release()
		if f.count("current_base_version") != 1 {
			t.Fatal("retry selected a new base")
		}
		if f.records.latest.Phase != webapi.ResetPhaseReady {
			t.Fatal(f.records.latest.Phase)
		}
	})
}

func TestResetLeaseTimeoutAndStorageFailure(t *testing.T) {
	for _, storage := range []bool{false, true} {
		t.Run(map[bool]string{false: "lease", true: "storage"}[storage], func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				f := newCloudFixture(t)
				f.flush = true
				a := f.access()
				if storage {
					f.records.writeFailure = webapi.ResetPhaseStopping
					a.Release()
				}
				start := time.Now()
				_, err := Reset(t.Context(), f, "failed")
				if err != nil {
					t.Fatal(err)
				}
				f.cloud.mu.Lock()
				task := f.cloud.machine.reset
				f.cloud.mu.Unlock()
				if task == nil {
					t.Fatal("missing task")
				}
				err = task.wait(t.Context())
				requireCloudError(t, err, Failed)
				if storage {
					if !errors.Is(err, io.ErrUnexpectedEOF) {
						t.Fatal(err)
					}
				} else {
					if time.Since(start) != 2*time.Second {
						t.Fatal(time.Since(start))
					}
					a.Release()
				}
				if f.count("reset") != 0 || f.phase() != webapi.CloudStateOff {
					t.Fatal("failed hold still rebuilt")
				}
			})
		})
	}
}

func TestStatusDoesNotAllocateAndGrowthChecksOwnership(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		status, err := Status(t.Context(), f)
		if err != nil || status.State != webapi.CloudStateUnallocated || len(f.calls) != 0 {
			t.Fatalf("unallocated status: %+v %v", status, err)
		}
		device, err := Device(t.Context(), f)
		if err != nil {
			t.Fatal(err)
		}
		status, err = Status(t.Context(), f)
		if err != nil || status.State != webapi.CloudStateOff || status.Volumes == nil ||
			status.Volumes.HomeBytes != 2048 {
			t.Fatalf("off status: %+v %v", status, err)
		}
		if f.count("wake") != 0 {
			t.Fatal("status woke Cloud")
		}
		for _, size := range []uint64{0, f.services.Tuning.HomeQuota + 1} {
			if err := GrowVolume(t.Context(), f, device.ID, runnerwire.VolumeNameHome, size); err == nil {
				t.Fatal("invalid growth accepted")
			}
		}
		if err := GrowVolume(t.Context(), f, "other-device", runnerwire.VolumeNameHome, 1024); err == nil {
			t.Fatal("foreign growth accepted")
		}
		if err := GrowVolume(t.Context(), f, device.ID, runnerwire.VolumeNameHome, 4096); err != nil {
			t.Fatal(err)
		}
		if f.count("grow_volume") != 1 {
			t.Fatal("growth count")
		}
		f.hook = func(call machinewire.Call) (string, error) {
			if call.Name() == "image_state" {
				return "", io.EOF
			}
			return "", nil
		}
		status, err = Status(t.Context(), f)
		if err != nil || status.Volumes != nil {
			t.Fatalf("manager failure leaked to status: %v", err)
		}
	})
}

func TestCloseLeavesHeldMachineForManagerReconcile(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		a := f.access()
		if err := Close(t.Context(), f); err != nil {
			t.Fatal(err)
		}
		if f.count("hibernate") != 0 {
			t.Fatal("shutdown interrupted held machine")
		}
		_, err := Access(t.Context(), f)
		requireCloudError(t, err, Closed)
		a.Release()
	})
}

func TestConversationHoldsReleaseOnPartialResetFailure(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.holdFailure = "second"
		held, err := holdReset(
			t.Context(),
			f,
			[]cloudUse{
				{id: "first", role: targetRole},
				{id: "second", role: targetRole},
				{id: "attached", role: attachedRole},
			},
			time.Second,
		)
		if held != nil || err == nil || f.holds != 0 {
			t.Fatalf("partial reset: %v %v holds=%d", held, err, f.holds)
		}
		// A provider role is held but its paired files remain accessible.
		f.holdFailure = ""
		held, err = holdReset(t.Context(), f, []cloudUse{{id: "provider", role: providerRole}}, time.Second)
		if err != nil {
			t.Fatal(err)
		}
		releaseHolds(held)
		if f.resetFiles[len(f.resetFiles)-1] {
			t.Fatal("provider's paired files were interrupted")
		}
	})
}

func TestPerOperationLeasePreventsIdleUntilReleased(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.flush = true
		a := f.access()
		lease, err := a.admission.PerOperation()
		if err != nil {
			t.Fatal(err)
		}
		a.Release()
		time.Sleep(12 * time.Second)
		if f.count("hibernate") != 0 {
			t.Fatal("Host operation lost admission")
		}
		lease.Release()
		f.waitCalls(t.Context(), "hibernate", 1)
		synctest.Wait()
		if state := f.cloud.machine.gate.State(); state.Demand != 0 || state.Maintenance != 0 || state.Reserved {
			t.Fatalf("leases leaked: %+v", state)
		}
	})
}

func TestStartupReconcilesAndRecoversWithoutBooting(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		device, err := Device(t.Context(), f)
		if err != nil {
			t.Fatal(err)
		}
		op := database.ManagedOperation{
			ID:          "interrupted",
			BaseVersion: "pinned-base",
			Phase:       webapi.ResetPhaseRebuilding,
		}
		if err := f.records.PutManagedOperation(t.Context(), device.ID, op); err != nil {
			t.Fatal(err)
		}
		store := &recoveryFixture{memoryRecords: f.records}
		if err := recoverResets(t.Context(), store, f.services); err != nil {
			t.Fatal(err)
		}
		if !store.exposesDeleted || f.count("reconcile") != 1 || f.count("reset") != 1 || f.count("wake") != 0 {
			t.Fatal("startup order or scope changed")
		}
		if f.calls[0].Name() != "reconcile" {
			t.Fatal("startup did not reconcile first")
		}
		reset, ok := f.calls[1].(*machinewire.Reset)
		if !ok || reset.Params.BaseVersion != "pinned-base" || reset.Params.OperationID != "interrupted" {
			t.Fatal("startup lost durable reset identity")
		}
		if f.records.latest.Phase != webapi.ResetPhaseFailed || f.records.latest.Error == nil ||
			*f.records.latest.Error != "Reset disks recovered; retry to start Cloud" {
			t.Fatalf("recovered status: %+v", f.records.latest)
		}
		if !reflect.DeepEqual(f.records.announced, []webapi.OperationID{"interrupted"}) {
			t.Fatal("reset not announced")
		}
		// The public entry point reaches that same control boundary and preserves its error.
		err = RecoverResets(t.Context(), f.control, f.services)
		if !errors.Is(err, database.ErrClosed) {
			t.Fatalf("closed storage: %v", err)
		}
	})
}

type recoveryFixture struct {
	*memoryRecords
	exposesDeleted bool
}

func (r *recoveryFixture) DeleteCloudExposes(context.Context) error {
	r.exposesDeleted = true
	return nil
}

func (r *recoveryFixture) UnfinishedManagedOperations(context.Context) ([]database.DeviceOperation, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	var result []database.DeviceOperation
	for _, op := range r.operations {
		if op.Phase != webapi.ResetPhaseReady && op.Phase != webapi.ResetPhaseFailed {
			result = append(result, database.DeviceOperation{Device: r.device.ID, Operation: op})
		}
	}
	return result, nil
}

func TestResetDuringBootJoinsItAndSharesItsPermit(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.flush = true
		f.services.Capacity = NewCapacity(1)
		booting := make(chan struct{})
		first := true
		f.hook = func(call machinewire.Call) (string, error) {
			if call.Name() == "wake" && first {
				first = false
				<-booting
			}
			return "", nil
		}
		waiter := make(chan *MachineAccess, 1)
		go func() { waiter <- f.access() }()
		f.waitCalls(t.Context(), "wake", 1)
		_, err := Reset(t.Context(), f, "during-boot")
		if err != nil {
			t.Fatal(err)
		}
		synctest.Wait()
		if f.count("reset") != 0 {
			t.Fatal("reset overtook boot")
		}
		close(booting)
		a := <-waiter
		a.Release()
		if f.count("wake") != 2 || f.count("reset") != 1 {
			t.Fatal("did not join reset after boot")
		}
		status, err := Status(t.Context(), f)
		if err != nil || status.Operation == nil || status.Operation.Phase != webapi.ResetPhaseReady ||
			status.State != webapi.CloudStateRunning {
			t.Fatalf("reset status: %+v %v", status, err)
		}
	})
}

func TestCheckpointFailureRetriesWithoutStoppingCloud(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.flush = true
		f.services.Tuning.CheckpointInterval = 2 * time.Second
		fail := true
		f.hook = func(call machinewire.Call) (string, error) {
			if call.Name() == "checkpoint" && fail {
				return "", errors.New("checkpoint refused")
			}
			return "", nil
		}
		a := f.access()
		f.waitCalls(t.Context(), "checkpoint", 1)
		synctest.Wait()
		if f.phase() != webapi.CloudStateRunning {
			t.Fatal("failed checkpoint stopped Cloud")
		}
		fail = false
		f.waitCalls(t.Context(), "checkpoint", 2)
		synctest.Wait()
		if time.Since(f.cloud.machine.started) != 3*time.Second {
			t.Fatal("checkpoint did not retry next sweep")
		}
		if f.cloud.machine.checkpoint != time.Now() {
			t.Fatal("successful checkpoint not recorded")
		}
		a.Release()
	})
}

func TestLifetimeCapTimeoutReopensAdmission(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.services.Tuning.LifetimeCap = 3 * time.Second
		a := f.access()
		time.Sleep(3 * time.Second)
		synctest.Wait()
		f.cloud.mu.Lock()
		retirement := f.cloud.machine.retirement
		f.cloud.mu.Unlock()
		if retirement == nil {
			t.Fatal("cap did not start")
		}
		if err := retirement.wait(t.Context()); err != nil {
			t.Fatal(err)
		}
		if f.count("hibernate") != 0 || f.phase() != webapi.CloudStateRunning {
			t.Fatal("timed-out reservation stopped machine")
		}
		a.Release()
		// Prevent a second cap from masking whether the timed-out reservation released.
		f.cloud.Stop()
		if reserved := f.cloud.machine.gate.TryReserve(); reserved == nil {
			t.Fatal("cap reservation leaked")
		} else {
			reserved.Release()
		}
	})
}

func TestFlushDeadlineStillSavesCloud(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		a := f.access()
		a.Release()
		start := time.Now()
		if err := Close(t.Context(), f); err != nil {
			t.Fatal(err)
		}
		if time.Since(start) != f.services.Tuning.SyncTimeout || f.count("hibernate") != 1 ||
			f.phase() != webapi.CloudStateOff {
			t.Fatalf("flush timeout prevented save: %s %s", time.Since(start), f.phase())
		}
	})
}

func TestPanickedBootFailsWaitersAndReleasesCapacity(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.services.Capacity = NewCapacity(1)
		f.records.panicRotate = true
		_, err := Access(t.Context(), f)
		requireCloudError(t, err, Failed)
		if err.Error() != "A transition of the Cloud ended without an answer" || f.phase() != webapi.CloudStateOff {
			t.Fatalf("panic escaped ownership: %v phase=%s", err, f.phase())
		}
		permit := f.services.Capacity.TryTake()
		if permit == nil {
			t.Fatal("panicked boot leaked permit")
		}
		permit.Release()
	})
}

func TestPanickedIdleSaveSettlesAndReleasesCapacity(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newCloudFixture(t)
		f.services.Capacity = NewCapacity(1)
		f.panicStop = true
		a := f.access()
		a.Release()
		time.Sleep(11 * time.Second)
		synctest.Wait()
		if f.phase() != webapi.CloudStateOff {
			t.Fatal("panicked retirement left a transition stuck")
		}
		f.cloud.mu.Lock()
		transition := f.cloud.machine.transition
		failure := f.cloud.machine.failure
		f.cloud.mu.Unlock()
		if transition != nil || failure == nil || *failure != "A transition of the Cloud ended without an answer" {
			t.Fatal("retirement did not settle its failure")
		}
		permit := f.services.Capacity.TryTake()
		if permit == nil {
			t.Fatal("panicked retirement leaked capacity")
		}
		permit.Release()
	})
}
