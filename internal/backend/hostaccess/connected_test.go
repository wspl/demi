package hostaccess

import (
	"context"
	"errors"
	"fmt"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// Cost: local SQLite/blob fixtures and in-process runner frames, no processes.
func TestConnectedNodeIdentityRefusesFilesAndWithHostHoldsGate(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	r := connectHost(t, s, device)
	identity, err := ConversationHostForNode(t.Context(), s, record.ID)
	if err != nil {
		t.Fatal(err)
	}
	if identity.Identity().Hostname != "connected" || identity.DefaultCWD() != "/work" {
		t.Fatal("node lost runner identity")
	}
	// Refusal must win without ever sending a filesystem request.
	requireFileRefused(t, r, identity)
	var escaped *remotehost.Host
	result := startHostOperation(t, func(ctx context.Context) (host.FileStat, error) {
		return WithHost(ctx, s, record.ID, nil, func(ctx context.Context, admitted *ConversationHost) (host.FileStat, error) {
			if !s.mu.TryLock() {
				t.Error("WithHost called operation under shard mutex")
			} else {
				s.mu.Unlock()
			}
			escaped = admitted.Host
			return admitted.Host.FS().Stat(ctx, "/work/file")
		})
	})
	var request *runnerwire.FSStat
	select {
	case frame := <-r.outgoing:
		message, err := runnerwire.DecodeInbound(frame)
		if err != nil {
			t.Fatal(err)
		}
		var ok bool
		request, ok = message.(*runnerwire.FSStat)
		if !ok {
			t.Fatalf("WithHost sent %T instead of stat", message)
		}
	case completed := <-result:
		t.Fatalf("file operation ended before runner reply: %v", completed.err)
	}
	gate := s.conversations.Slot(record.ID).FileGate()
	if held := gate.TryReserve(); held != nil {
		held.Release()
		t.Error("file gate released before runner answered")
	}
	if transition, err := HoldForTransition(t.Context(), s, record.ID, nil); err == nil {
		transition.Release()
		t.Error("transition entered while file operation awaited its reply")
	}
	r.send(t, &runnerwire.FSOK{ID: request.ID, Result: &runnerwire.FSStatResult{Value: runnerwire.FileStat{IsFile: true, Size: 7}}})
	completed := <-result
	if completed.err != nil || completed.value.Size != 7 {
		t.Fatal(completed)
	}
	held := gate.TryReserve()
	if held == nil {
		t.Fatal("file gate kept after file operation")
	}
	held.Release()
	requireFileRefused(t, r, escaped)
}

func TestConnectedDownloadRevocationLeavesReaderWithEdge(t *testing.T) {
	for _, end := range []string{"transition", "shutdown", "device", "edge"} {
		t.Run(end, func(t *testing.T) {
			s := newTestShard(t)
			device := s.paired(t, "laptop")
			record := s.target(t, s.conversation(t), device, "/work")
			r := connectHost(t, s, device)
			result := startHostOperation(t, func(ctx context.Context) (Download, error) {
				return OpenDownload(ctx, s, record.ID, DownloadRequest{Path: "/file"})
			})
			r.stat(t, 100)
			read, ok := r.next(t).(*runnerwire.FSReadFile)
			if !ok {
				t.Fatal("expected readFile")
			}
			r.send(t, &runnerwire.FSOK{ID: read.ID, Result: &runnerwire.FSReadFileResult{}})
			opened := <-result
			if opened.err != nil {
				t.Fatal(opened.err)
			}
			download := opened.value.(*DownloadStream)
			t.Cleanup(func() {
				download.Lease.Release()
				if err := download.Body.Close(context.Background()); err != nil {
					t.Error(err)
				}
			})
			if held := s.conversations.Slot(record.ID).FileGate().TryReserve(); held != nil {
				held.Release()
				t.Fatal("download dropped admission before edge finished")
			}
			switch end {
			case "transition":
				held, err := HoldForTransition(t.Context(), s, record.ID, nil)
				if err != nil {
					t.Fatal(err)
				}
				held.Release()
			case "shutdown":
				if err := s.conversations.Close(t.Context()); err != nil {
					t.Fatal(err)
				}
			case "device":
				if err := r.connection.Close(t.Context()); err != nil {
					t.Fatal(err)
				}
				// The edge observes the pipe failure and releases its lease, by contract.
				if _, err := download.Body.Next(t.Context()); err == nil {
					t.Fatal("device loss did not fail edge read")
				}
				download.Lease.Release()
			case "edge":
				download.Lease.Release()
			}
			<-download.Lease.Context().Done()
			// Closing drains the owned admission; it does not require the edge to close
			// its reader or call Release after transition/shutdown revocation.
			closed, err := s.conversations.Slot(record.ID).Transfers().Close(t.Context())
			if err != nil {
				t.Fatal(err)
			}
			closed.Release()
			held := s.conversations.Slot(record.ID).FileGate().TryReserve()
			if held == nil {
				t.Fatal("ended download retained file gate")
			}
			held.Release()
		})
	}
}

func TestConnectedUploadRevocationFailsPipeAndDrains(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	r := connectHost(t, s, device)
	result := startHostOperation(t, func(ctx context.Context) (Upload, error) {
		return UploadFile(ctx, s, record.ID, "/file", true)
	})
	r.stat(t, 5)
	write, ok := r.next(t).(*runnerwire.FSWriteFile)
	if !ok {
		t.Fatal("expected writeFile")
	}
	opened := <-result
	if opened.err != nil {
		t.Fatal(opened.err)
	}
	upload := opened.value.(*OpenUpload)
	defer upload.Lease.Release()
	pipe, ok := s.pipes.Pipe(write.Input.ID)
	if !ok {
		t.Fatal("upload pipe missing")
	}
	hold, err := HoldForTransition(t.Context(), s, record.ID, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer hold.Release()
	if err := upload.Written(t.Context()); err == nil {
		t.Fatal("revoked upload reported success")
	}
	if pipe.Failure() == nil || upload.Lease.Context().Err() == nil {
		t.Fatal("revocation did not fail pipe and edge lease")
	}
	if err := upload.Writer.Write(t.Context(), []byte("late")); err == nil {
		t.Fatal("edge wrote after upload revoked")
	}

}

func TestConnectedStreamRetainsActivityWithoutFilesAndEnds(t *testing.T) {
	for _, end := range []string{"transition", "shutdown", "device", "edge"} {
		t.Run(end, func(t *testing.T) {
			s := newTestShard(t)
			device := s.paired(t, "laptop")
			record := s.target(t, s.conversation(t), device, "/work")
			r := connectHost(t, s, device)
			binding := testServiceBinding()
			result := startHostOperation(t, func(ctx context.Context) (*UserStream, error) {
				return OpenUserStream(ctx, s, record.ID, binding)
			})
			request, ok := r.next(t).(*runnerwire.ServiceOpen)
			if !ok {
				t.Fatal("expected service open")
			}
			r.send(t, &runnerwire.ServiceOpened{StreamID: request.StreamID})
			opened := <-result
			if opened.err != nil {
				t.Fatal(opened.err)
			}
			stream := opened.value
			defer stream.Lease.Release()
			defer func() { _ = stream.FromHost.Close(context.Background()) }()
			slot := s.conversations.Slot(record.ID)
			if slot.Streams().State().Demand != 1 {
				t.Fatal("stream did not retain activity")
			}
			hold := slot.FileGate().TryReserve()
			if hold == nil {
				t.Fatal("stream retained file gate")
			}
			hold.Release()
			switch end {
			case "transition":
				transition, err := HoldForTransition(t.Context(), s, record.ID, nil)
				if err != nil {
					t.Fatal(err)
				}
				transition.Release()
			case "shutdown":
				if err := s.conversations.Close(t.Context()); err != nil {
					t.Fatal(err)
				}
			case "device":
				if err := r.connection.Close(t.Context()); err != nil {
					t.Fatal(err)
				}
			case "edge":
				stream.Lease.Release()
			}
			<-stream.Lease.Context().Done()
			closed, err := slot.Transfers().Close(t.Context())
			if err != nil {
				t.Fatal(err)
			}
			closed.Release()
			if stream.Lease.Context().Err() == nil || slot.Streams().State().Demand != 0 {
				t.Fatal("transition did not end stream activity and lease")
			}
			if _, err := stream.FromHost.Next(t.Context()); err == nil {
				t.Fatal("ended stream pipe stayed open")
			}
		})
	}
}

func testServiceBinding() ServiceBinding {
	return ServiceBinding{Package: commandwire.PackageDescriptor{ID: "test.service", Version: "1", ProtocolVersion: 1, Operations: []string{"view"}, Targets: map[string]commandwire.PackageArtifact{}}, Operation: "view"}
}

func TestConnectedUserCallActivityAndTransitionCancellation(t *testing.T) {
	for _, kind := range []UserCallKind{Looks, Operates, Starts} {
		t.Run(fmt.Sprint(kind), func(t *testing.T) {
			s := newTestShard(t)
			device := s.paired(t, "laptop")
			record := s.target(t, s.conversation(t), device, "/work")
			r := connectHost(t, s, device)
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			result := startHostOperation(t, func(context.Context) ([]byte, error) {
				return UserCall(ctx, s, record.ID, kind, ServiceCall{Binding: testServiceBinding(), MaxBytes: 100})
			})
			request, ok := r.next(t).(*runnerwire.ServiceOpen)
			if !ok {
				t.Fatal("expected service open")
			}
			r.send(t, &runnerwire.ServiceOpened{StreamID: request.StreamID})
			slot := s.conversations.Slot(record.ID)
			state := slot.FileGate().State()
			if kind == Looks && (!state.LastDemandEnd.IsZero() || state.Demand != 0) {
				t.Fatal("look counted as demand", state)
			}
			if kind == Operates && state.LastDemandEnd.IsZero() {
				t.Fatal("operation did not count as activity")
			}
			transition, err := HoldForTransition(t.Context(), s, record.ID, nil)
			if kind == Starts {
				var refused *ChangeRefusal
				if !errors.As(err, &refused) || refused.Kind != ChangeTurnInFlight {
					if transition != nil {
						transition.Release()
					}
					t.Fatal("starting call did not hold file admission", err)
				}
				cancel()
			} else {
				if err != nil {
					t.Fatal(err)
				}
				transition.Release()
			}
			if completed := <-result; completed.err == nil {
				t.Fatal("ended call reported success")
			}
			if s.idle < 2 {
				t.Fatal("admitted call omitted idle watch")
			}
		})
	}
}

func TestConnectedLifecycleFailureStillCommitsAfterRequesterLeaves(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	r := connectHost(t, s, device)
	held, err := HoldForTransition(t.Context(), s, record.ID, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer held.Release()
	result := startHostOperation(t, func(ctx context.Context) (struct{}, error) {
		cancelled, cancel := context.WithCancel(ctx)
		cancel()
		return struct{}{}, Archive(cancelled, s, record)
	})
	request, ok := r.next(t).(*runnerwire.ConversationRelease)
	if !ok {
		t.Fatal("expected conversation release")
	}
	before, err := OwnedConversation(t.Context(), s, record.ID)
	if err != nil || before.Archived {
		t.Fatal("archive committed before release", err)
	}
	r.send(t, &runnerwire.ConversationReleased{ID: request.ID, Error: new("scripted release failure")})
	if completed := <-result; completed.err != nil {
		t.Fatal(completed.err)
	}
	after, err := OwnedConversation(t.Context(), s, record.ID)
	if err != nil || !after.Archived {
		t.Fatal("release failure prevented commit", err)
	}
}

// requireFileRefused fails at the wire boundary if a public Host reaches files
// without an admission; cancellation still joins a planted-defect request.
func requireFileRefused(t *testing.T, r *boundRunner, target *remotehost.Host) {
	t.Helper()
	result := startHostOperation(t, func(ctx context.Context) (host.FileStat, error) {
		return target.FS().Stat(ctx, "/work/file")
	})
	select {
	case completed := <-result:
		if completed.err == nil {
			t.Fatal("unadmitted file operation succeeded")
		}
	case <-r.outgoing:
		t.Fatal("unadmitted file operation reached runner")
	}
}
