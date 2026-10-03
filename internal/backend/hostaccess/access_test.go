package hostaccess

import (
	"context"
	"errors"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/gates/gatestest"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

func TestCloudWaitReleasesFilesAndRechecksChangedTarget(t *testing.T) {
	for _, change := range []string{"switch", "archive", "detach"} {
		t.Run(change, func(t *testing.T) {
			s := newTestShard(t)
			record := s.conversation(t)
			cloudDevice, err := s.control.ManagedDeviceOrCreate(t.Context(), s.owner)
			if err != nil {
				t.Fatal(err)
			}
			laptop := s.paired(t, "laptop")
			var named *webapi.DeviceID
			if change == "detach" {
				record = s.target(t, record, laptop, "/laptop")
				if _, err := s.control.ChangeConversation(t.Context(), record.ID, &database.RecordAttach{Host: database.AttachedHostRecord{Device: cloudDevice.ID, Name: "cloud"}}); err != nil {
					t.Fatal(err)
				}
				named = &cloudDevice.ID
			}
			waiting := make(chan struct{})
			proceed := make(chan struct{})
			var released atomic.Int32
			s.conversations.cloudAdmission = func(ctx context.Context, _ cloud.CloudShard, device database.DeviceRecord) (*cloudHold, error) {
				close(waiting)
				select {
				case <-proceed:
				case <-ctx.Done():
					return nil, ctx.Err()
				}
				return &cloudHold{device: device.ID, release: func() { released.Add(1) }}, nil
			}
			outcome := make(chan error, 1)
			var ran atomic.Int32
			go func() {
				_, err := WithHost(t.Context(), s, record.ID, named, func(_ context.Context, h *ConversationHost) (struct{}, error) {
					ran.Add(1)
					if h.Root != "/laptop" {
						t.Errorf("stale root %q", h.Root)
					}
					return struct{}{}, nil
				})
				outcome <- err
			}()
			<-waiting
			hold, err := HoldForTransition(t.Context(), s, record.ID, nil)
			if err != nil {
				close(proceed)
				<-outcome
				t.Fatal("Cloud wait retained the file gate", err)
			}
			switch change {
			case "switch":
				err = SwitchTarget(t.Context(), s, record, &webapi.ConversationTargetDevice{DeviceID: laptop.ID, Path: "/laptop"})
			case "archive":
				err = Archive(t.Context(), s, record)
			case "detach":
				err = Detach(t.Context(), s, record, cloudDevice.ID)
			}
			hold.Release()
			close(proceed)
			admissionErr := <-outcome
			if err != nil {
				t.Fatal(err)
			}
			if change == "switch" {
				if admissionErr != nil || ran.Load() != 1 {
					t.Fatalf("switch: %v, runs=%d", admissionErr, ran.Load())
				}
			} else {
				var refused Refusal
				want := Archived
				if change == "detach" {
					want = NotAttached
				}
				if !errors.As(admissionErr, &refused) || refused != want || ran.Load() != 0 {
					t.Fatalf("admission %v, runs=%d", admissionErr, ran.Load())
				}
			}
			if released.Load() != 1 {
				t.Fatal("Cloud hold not released exactly once")
			}
		})
	}
}

func TestAdmissionRefusalsAndDispatchedWorkRunsOnce(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	cases := []struct {
		name  string
		id    webapi.ConversationID
		named *webapi.DeviceID
		want  Refusal
	}{
		{"not attached", record.ID, new(webapi.DeviceID("00000000-0000-4000-8000-000000000099")), NotAttached},
	}
	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			admitted, err := AdmitHost(t.Context(), s, tt.id, tt.named)
			if admitted != nil {
				admitted.Release()
			}
			var refusal Refusal
			if !errors.As(err, &refusal) || refusal != tt.want {
				t.Fatal(err)
			}
		})
	}
	if _, err := OwnedConversation(t.Context(), s, "00000000-0000-4000-8000-000000000099"); err == nil {
		t.Fatal("missing conversation found")
	}
	runs := 0
	cause := &host.Error{Kind: host.Offline, Message: "runner disconnected"}
	_, err := WithHost(t.Context(), s, record.ID, nil, func(context.Context, *ConversationHost) (int, error) {
		runs++
		return 0, cause
	})
	if !errors.Is(err, cause) || runs != 1 {
		t.Fatalf("dispatched: %v, runs=%d", err, runs)
	}
	if _, err := s.control.ChangeConversation(t.Context(), record.ID, &database.RecordArchived{Archived: true}); err != nil {
		t.Fatal(err)
	}
	admitted, err := AdmitHost(t.Context(), s, record.ID, nil)
	if admitted != nil {
		admitted.Release()
	}
	var refused Refusal
	if !errors.As(err, &refused) || refused != Archived {
		t.Fatal(err)
	}
}

func TestCancelledFileGateWaitDoesNotDispatch(t *testing.T) {
	s := newTestShard(t)
	record := s.target(t, s.conversation(t), s.paired(t, "laptop"), "/work")
	held, err := s.conversations.Slot(record.ID).FileGate().Reserve(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	defer held.Release()
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	_, err = WithHost(ctx, s, record.ID, nil, func(context.Context, *ConversationHost) (int, error) {
		t.Error("cancelled wait dispatched")
		return 0, nil
	})
	if !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
}

func TestNoWakeOperationsRefuseStoppedCloudAndOfflineDevice(t *testing.T) {
	for _, cloudTarget := range []bool{true, false} {
		t.Run(map[bool]string{true: "Cloud", false: "paired"}[cloudTarget], func(t *testing.T) {
			s := newTestShard(t)
			record := s.conversation(t)
			if !cloudTarget {
				record = s.target(t, record, s.paired(t, "laptop"), "/work")
			}
			s.mu.Lock()
			before := s.idle
			s.mu.Unlock()
			for _, kind := range []UserCallKind{Looks, Operates} {
				_, err := UserCall(t.Context(), s, record.ID, kind, ServiceCall{})
				if err == nil {
					t.Fatal("no-wake call reached unavailable Host")
				}
			}
			_, err := OpenUserStream(t.Context(), s, record.ID, ServiceBinding{})
			if err == nil {
				t.Fatal("stream reached unavailable Host")
			}
			_, err = ReadFiles(t.Context(), s, record.ID, nil)
			var readError *ReadFilesError
			if !errors.As(err, &readError) || readError.Kind != ReadFilesNotRunning {
				t.Fatal(err)
			}
			s.mu.Lock()
			after := s.idle
			s.mu.Unlock()
			if after != before || s.conversations.Slot(record.ID).Transfers().AnyOpen() {
				t.Fatal("refused admission retained activity or registration")
			}
			managed, err := s.control.ManagedDevice(t.Context(), s.owner)
			if err != nil || managed != nil {
				t.Fatalf("no-wake allocated Cloud: %v, %v", managed, err)
			}
		})
	}
}

func TestTransitionCommitIgnoresDepartedRequesterAndKeepsConflict(t *testing.T) {
	s := newTestShard(t)
	record := s.conversation(t)
	device := s.paired(t, "laptop")
	hold, err := HoldForTransition(t.Context(), s, record.ID, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer hold.Release()
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	target := &webapi.ConversationTargetDevice{DeviceID: device.ID, Path: "/work"}
	if err := SwitchTarget(ctx, s, record, target); err != nil {
		t.Fatal(err)
	}
	current, err := OwnedConversation(t.Context(), s, record.ID)
	if err != nil {
		t.Fatal(err)
	}
	if selected, ok := current.Target.(*webapi.ConversationTargetDevice); !ok || selected.Path != "/work" {
		t.Fatalf("commit abandoned: %#v", current.Target)
	}
	s.mu.Lock()
	idle := s.idle
	s.mu.Unlock()
	if idle != 1 {
		t.Fatal("committed switch did not restart idle tracking")
	}
	var conflict *ChangeRefusal
	if err := SwitchTarget(t.Context(), s, record, target); !errors.As(err, &conflict) || conflict.Kind != ChangeConflict {
		t.Fatalf("stale switch = %v", err)
	}
	if err := Archive(ctx, s, current); err != nil {
		t.Fatal(err)
	}
	archived, err := OwnedConversation(t.Context(), s, record.ID)
	if err != nil || !archived.Archived {
		t.Fatalf("archive: %v, %v", archived, err)
	}
}

func TestOwnershipCanonicalIDsAndRevokedTargets(t *testing.T) {
	s := newTestShard(t)
	id := webapi.ConversationID("abcdef00-0000-4000-8000-000000000001")
	if _, err := s.control.CreateConversation(t.Context(), s.owner, id); err != nil {
		t.Fatal(err)
	}
	record, err := OwnedConversation(t.Context(), s, "ABCDEF00-0000-4000-8000-000000000001")
	if err != nil || record.ID != id {
		t.Fatalf("canonical ID: %v, %v", record, err)
	}
	owner := s.owner
	s.owner = "another-user"
	_, err = OwnedConversation(t.Context(), s, id)
	s.owner = owner
	var missing *Error
	if !errors.As(err, &missing) || missing.Kind != AccessMissing {
		t.Fatal(err)
	}
	device := s.paired(t, "laptop")
	record = s.target(t, record, device, "/work")
	if err := s.control.DeleteDevice(t.Context(), device.ID); err != nil {
		t.Fatal(err)
	}
	admitted, err := AdmitHost(t.Context(), s, record.ID, nil)
	if admitted != nil {
		admitted.Release()
	}
	var refusal Refusal
	if !errors.As(err, &refusal) || refusal != DeviceGone {
		t.Fatal(err)
	}
}

func TestRunJobRejectsStaleIdentityAndAccountsCompletion(t *testing.T) {
	s := newTestShard(t)
	record := s.target(t, s.conversation(t), s.paired(t, "laptop"), "/work")
	identity, err := ConversationHostForNode(t.Context(), s, record.ID)
	if err != nil {
		t.Fatal(err)
	}
	runs := 0
	if err := RunJob(t.Context(), s, record.ID, identity.Key(), func(context.Context) error {
		runs++
		return errors.New("job failed")
	}); err == nil {
		t.Fatal("job failure lost")
	}
	s.mu.Lock()
	jobs := s.jobs
	s.mu.Unlock()
	if runs != 1 || jobs != 1 {
		t.Fatalf("runs=%d, completions=%d", runs, jobs)
	}
	record = s.target(t, record, s.paired(t, "other"), "/elsewhere")
	// The old Host stays attached, but its attached cwd differs from this identity.
	if err := RunJob(t.Context(), s, record.ID, host.Key(string(identity.Key())+"/stale"), func(context.Context) error {
		runs++
		return nil
	}); err == nil {
		t.Fatal("stale identity ran")
	}
	if runs != 1 {
		t.Fatal("stale callback dispatched")
	}
}

func TestTransitionCancelsTransferWaitingForFileAdmission(t *testing.T) {
	s := newTestShard(t)
	record := s.target(t, s.conversation(t), s.paired(t, "laptop"), "/work")
	slot := s.conversations.Slot(record.ID)
	files, err := slot.FileGate().Reserve(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	defer files.Release()
	done := make(chan error, 1)
	go func() {
		_, err := OpenDownload(t.Context(), s, record.ID, DownloadRequest{Path: "/file"})
		done <- err
	}()
	for {
		waiting, changed := gatestest.Waiting(slot.FileGate())
		if waiting == 1 {
			break
		}
		select {
		case <-changed:
		case <-t.Context().Done():
			t.Fatal(t.Context().Err())
		}
	}
	if !slot.Transfers().AnyOpen() {
		t.Error("transfer waited before registration")
	}
	held, err := HoldForTransition(t.Context(), s, record.ID, nil)
	if held != nil {
		held.Release()
	}
	var busy *ChangeRefusal
	if !errors.As(err, &busy) || busy.Kind != ChangeTurnInFlight {
		t.Error("existing file reservation was not busy", err)
	}
	var refused Refusal
	if err := <-done; !errors.As(err, &refused) || refused != Busy {
		t.Fatalf("waiting download: %v", err)
	}
	if slot.Transfers().AnyOpen() {
		t.Fatal("cancelled admission still registered")
	}
	fresh, err := slot.Transfers().Open()
	if err != nil {
		t.Fatal("failed transition left admission closed", err)
	}
	fresh.Release()
}

// Cost: temporary SQLite and blob directories; all ordering uses events.
func TestShutdownRefusesNewAdmissionAndJoinsDispatchedOperation(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	entered := make(chan struct{})
	finish := make(chan struct{})
	operationDone := make(chan error, 1)
	go func() {
		_, err := WithHost(t.Context(), s, record.ID, nil, func(context.Context, *ConversationHost) (struct{}, error) {
			close(entered)
			<-finish
			return struct{}{}, nil
		})
		operationDone <- err
	}()
	<-entered
	closed := make(chan error, 1)
	go func() { closed <- s.conversations.Close(context.Background()) }()
	<-s.conversations.ctx.Done()
	_, err := AdmitHost(t.Context(), s, record.ID, nil)
	var refused *Error
	if !errors.As(err, &refused) || refused.Kind != AccessCancelled {
		t.Errorf("shutdown admitted work: %v", err)
	}
	select {
	case err := <-closed:
		closed <- err
		t.Errorf("shutdown returned with an operation still running: %v", err)
	default:
	}
	close(finish)
	if err := <-operationDone; err != nil {
		t.Fatal(err)
	}
	if err := <-closed; err != nil {
		t.Fatal(err)
	}
	if _, err := s.conversations.Slot(record.ID).Transfers().Open(); err == nil {
		t.Fatal("shutdown reopened transfers")
	}
}
