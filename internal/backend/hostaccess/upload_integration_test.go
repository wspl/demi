package hostaccess

import (
	"context"
	"strings"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/types"
)

// Cost: local database/blob IO and in-process frames. A queued reservation makes
// accidental nested admission deadlock before any filesystem request is sent.
func TestConnectedUploadUsesExistingAdmissionWithTransitionQueued(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newTestShard(t)
		device := s.paired(t, "laptop")
		record := s.target(t, s.conversation(t), device, "/work")
		r := connectHost(t, s, device)
		blob, err := s.blobs.Put(t.Context(), []byte("attachment text"))
		if err != nil {
			t.Fatal(err)
		}
		upload, err := s.control.CreateAttachment(t.Context(), s.owner, "text/plain", 15, blob, nil)
		if err != nil {
			t.Fatal(err)
		}
		admitted, err := AdmitHost(t.Context(), s, record.ID, nil)
		if err != nil {
			t.Fatal(err)
		}
		defer admitted.Release()
		gate := s.conversations.Slot(record.ID).FileGate()
		startHostOperation(t, func(ctx context.Context) (struct{}, error) {
			hold, err := gate.Reserve(ctx)
			if err != nil {
				return struct{}{}, err
			}
			defer hold.Release()
			<-ctx.Done()
			return struct{}{}, nil
		})
		synctest.Wait()
		result := startHostOperation(t, func(ctx context.Context) ([]types.UserContentBlock, error) {
			content, _, err := ResolveUpload(ctx, s, record.ID, &admitted.Host, string(upload.ID), "notes.txt")
			return content, err
		})
		first := nextHostMessage[*runnerproto.FSExists](t, r)
		if !strings.HasSuffix(first.Path, "/notes.txt") {
			t.Fatal(first.Path)
		}
		r.send(t, &runnerproto.FSOK{ID: first.ID, Result: &runnerproto.FSExistsResult{Value: true}})
		second := nextHostMessage[*runnerproto.FSExists](t, r)
		expected := "/home/test/.demi/attachments/" + string(record.ID) + "/notes-2.txt"
		if second.Path != expected {
			t.Fatal(second.Path)
		}
		r.send(t, &runnerproto.FSOK{ID: second.ID, Result: &runnerproto.FSExistsResult{Value: false}})
		path, data := receiveHostWrite(t, s, r, device)
		if path != expected || string(data) != "attachment text" {
			t.Fatal(path, string(data))
		}
		completed := <-result
		if completed.err != nil || len(completed.value) == 0 {
			t.Fatal(completed)
		}
	})
}

// Cost: local storage and in-process runner frames; no subprocess or wall-time wait.
func TestConnectedUploadCompletionKeepsEdgeLeaseLive(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	r := connectHost(t, s, device)
	result := startHostOperation(t, func(ctx context.Context) (*OpenUpload, error) {
		return UploadFile(ctx, s, record.ID, "/notes.md", true)
	})
	r.stat(t, 5)
	opened := <-result
	if opened.err != nil {
		t.Fatal(opened.err)
	}
	upload := opened.value
	defer upload.Lease.Release()
	written := startHostOperation(t, func(ctx context.Context) (struct{}, error) {
		if err := upload.Writer.Write(ctx, []byte("first")); err != nil {
			return struct{}{}, err
		}
		upload.Writer.End()
		return struct{}{}, upload.Written(ctx)
	})
	path, data := receiveHostWrite(t, s, r, device)
	if path != "/notes.md" || string(data) != "first" {
		t.Fatal(path, string(data))
	}
	if completed := <-written; completed.err != nil {
		t.Fatal(completed.err)
	}
	// Wait for admission cleanup too: publishing Written just before cancelling
	// the lease would still race the edge's completion check.
	hold, err := s.conversations.Slot(record.ID).FileGate().Reserve(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	defer hold.Release()
	if err := upload.Lease.Context().Err(); err != nil {
		t.Fatalf("successful upload revoked the edge lease: %v", err)
	}
	if err := upload.Written(upload.Lease.Context()); err != nil {
		t.Fatal(err)
	}
	upload.Lease.Release()
	if err := upload.Lease.Context().Err(); err != nil {
		t.Fatalf("edge release revoked upload: %v", err)
	}
}
