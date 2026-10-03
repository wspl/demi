package backend_test

import (
	"bytes"
	"net/http"
	"testing"

	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
	"github.com/wspl/demi/internal/webapi"
)

// Two local uploads; no model or runner. Object counts pin deduplication.
func TestRepeatedUploadSendsNoObjectBytes(t *testing.T) {
	ctx, h := filesHarness(t)
	counts := &blobstest.ObjectCounts{}
	h.Objects = counts
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	png := storetest.PNG(4, 3, 0)
	upload := func(name string) webapi.AttachmentDTO {
		t.Helper()
		response, err := b.Response(ctx, "POST", "/api/attachments?name="+name, &s, http.Header{"Content-Type": []string{"image/png"}}, bytes.NewReader(png))
		wireMust(t, err)
		a, err := backendtest.ReadAnswer(ctx, response)
		wireMust(t, err)
		filesStatus(t, a, 201)
		decoded, err := webapi.DecodeAttachmentAnswer(a.Body)
		wireMust(t, err)
		return decoded.Attachment
	}
	before := counts.Tally()
	first := upload("shot.png")
	stored := counts.Tally().Since(before)
	if stored.Puts != 1 || stored.BytesPut != uint64(len(png)) {
		t.Fatalf("first upload: %+v", stored)
	}
	before = counts.Tally()
	again := upload("copy.png")
	repeated := counts.Tally().Since(before)
	if again.Sha256 != first.Sha256 {
		t.Fatal("same bytes have different hash")
	}
	if repeated.Puts != 0 || repeated.BytesPut != 0 || repeated.Heads != 1 {
		t.Fatalf("repeated upload: %+v", repeated)
	}
	wireMust(t, b.Close(ctx))
}
