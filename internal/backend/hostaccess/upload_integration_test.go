package hostaccess

import (
	"context"
	"strings"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/runnerwire"
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
		result := startHostOperation(t, func(ctx context.Context) ([]core.UserContentBlock, error) {
			content, _, err := ResolveUpload(ctx, s, record.ID, &admitted.Host, string(upload.ID), "notes.txt")
			return content, err
		})
		first := nextHostMessage[*runnerwire.FSExists](t, r)
		if !strings.HasSuffix(first.Path, "/notes.txt") {
			t.Fatal(first.Path)
		}
		r.send(t, &runnerwire.FSOK{ID: first.ID, Result: &runnerwire.FSExistsResult{Value: true}})
		second := nextHostMessage[*runnerwire.FSExists](t, r)
		expected := "/home/test/.demi/attachments/" + string(record.ID) + "/notes-2.txt"
		if second.Path != expected {
			t.Fatal(second.Path)
		}
		r.send(t, &runnerwire.FSOK{ID: second.ID, Result: &runnerwire.FSExistsResult{Value: false}})
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
