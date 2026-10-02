package store_test

import (
	"bytes"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/core"
)

var brokenPNG = []byte{0x89, 'P', 'N', 'G', 13, 10, 26, 10, 0, 255, 254, 1}

func TestUploadTypeAndOpening(t *testing.T) {
	for _, tc := range []struct {
		sent string
		data []byte
		want string
	}{{"application/octet-stream", brokenPNG, "image/png"}, {"application/octet-stream", []byte("%PDF-1.7 ..."), "application/pdf"}, {"text/csv", []byte("a,b\n1,2"), "text/csv"}} {
		if got := store.UploadMediaType(tc.sent, tc.data); got != tc.want {
			t.Fatalf("media type: %q", got)
		}
	}
	for _, tc := range []struct {
		name, media string
		want        bool
	}{{"notes.MD", "application/octet-stream", true}, {"data", "text/plain", true}, {"photo.png", "image/png", false}} {
		if got := store.IsText(tc.name, tc.media); got != tc.want {
			t.Fatalf("text classification: %s", tc.name)
		}
	}
	for _, tc := range []struct {
		data []byte
		want string
	}{
		{[]byte("\r\n  \n first\r\nsecond\rthird"), "first\nsecond\nthird"},
		{[]byte(strings.Repeat("字", 300)), strings.Repeat("字", 160)},
		{[]byte{' ', 0xe2, 0x82}, "\ufffd"},
		{append(bytes.Repeat([]byte(" "), 4095), 0xe2, 0x82, 0xac), "\ufffd"},
	} {
		if got := store.Snippet(tc.data); got != tc.want {
			t.Fatalf("snippet: %q, want %q", got, tc.want)
		}
	}
}

// testUpload describes the uploaded original without having the fake store put it.
func testUpload(t *testing.T, name, media string, data []byte) store.Upload {
	t.Helper()
	blobs := storetest.NewMemoryBlobs()
	blob, err := blobs.Put(t.Context(), data)
	if err != nil {
		t.Fatal(err)
	}
	return store.Upload{Name: name, Path: "/home/demi/.demi/attachments/c1/file", MediaType: media, SHA256: blob, Bytes: data}
}

func TestUploadNativeMediumThenRecord(t *testing.T) {
	blobs := storetest.NewMemoryBlobs()
	png := storetest.PNG(4, 3, 1)
	upload := testUpload(t, "tiny.png", "image/png", png)
	blocks, held, err := store.UploadBlocks(t.Context(), upload, blobs)
	if err != nil {
		t.Fatal(err)
	}
	if len(blocks) != 2 {
		t.Fatalf("image blocks: %v", blocks)
	}
	image, ok := blocks[0].(*core.UserImage)
	if !ok {
		t.Fatalf("first block: %T", blocks[0])
	}
	ref := image.Source.(*core.MediaSourceRef)
	if ref.Ref != upload.SHA256 || ref.MediaType != "image/png" {
		t.Fatalf("image reference: %#v", ref)
	}
	record := blocks[1].(*core.UserAttachment)
	if record.Snippet != nil {
		t.Fatal("image has text snippet")
	}
	if blobs.Holds(upload.SHA256) {
		t.Fatal("unchanged upload stored again")
	}
	view, missing := store.NewModelView(0, []core.Block{&core.UserBlock{Content: blocks}}, held)
	if len(missing) != 0 || !bytes.Equal(view.Held(ref.Ref).(*store.HeldBytes).Bytes, png) {
		t.Fatal("image bytes not held")
	}
	pdf, _, err := store.UploadBlocks(t.Context(), testUpload(t, "paper.pdf", "application/pdf", []byte("%PDF-1.7")), blobs)
	if err != nil || len(pdf) != 2 {
		t.Fatalf("PDF blocks: %v %v", pdf, err)
	}
	if _, ok := pdf[0].(*core.UserDocument); !ok {
		t.Fatalf("PDF medium: %T", pdf[0])
	}
	if _, ok := pdf[1].(*core.UserAttachment); !ok {
		t.Fatalf("PDF record: %T", pdf[1])
	}
	text, held, err := store.UploadBlocks(t.Context(), testUpload(t, "notes.txt", "text/plain", []byte("\n hello")), blobs)
	if err != nil || len(text) != 1 {
		t.Fatalf("text blocks: %v %v", text, err)
	}
	record = text[0].(*core.UserAttachment)
	if record.Name != "notes.txt" || record.SizeBytes != 7 || record.Snippet == nil || *record.Snippet != "hello" {
		t.Fatalf("text record: %#v", record)
	}
	// An unrelated image still needs reading: text brought no held bytes.
	_, missing = store.NewModelView(0, []core.Block{&core.UserBlock{Content: blocks}}, held)
	if len(missing) != 1 {
		t.Fatal("text held media")
	}
	if got := store.Unavailable("upload-9").(*core.UserText).Text; got != "[attachment upload-9 is not available]" {
		t.Fatal(got)
	}
}

func TestUploadedImageFitsOrStaysRecord(t *testing.T) {
	blobs := storetest.NewMemoryBlobs()
	upload := testUpload(t, "wide.png", "image/png", storetest.PNG(2400, 10, 1))
	blocks, held, err := store.UploadBlocks(t.Context(), upload, blobs)
	if err != nil || len(blocks) != 2 {
		t.Fatalf("wide upload: %v %v", blocks, err)
	}
	source := blocks[0].(*core.UserImage).Source.(*core.MediaSourceRef)
	if source.Ref == upload.SHA256 || source.MediaType != "image/png" {
		t.Fatalf("fitted reference: %#v", source)
	}
	data, found, err := blobs.Read(t.Context(), source.Ref)
	if err != nil || !found {
		t.Fatalf("fitted blob: %v %v", found, err)
	}
	view, missing := store.NewModelView(0, []core.Block{&core.UserBlock{Content: blocks}}, held)
	if len(missing) != 0 || !bytes.Equal(view.Held(source.Ref).(*store.HeldBytes).Bytes, data) {
		t.Fatal("wrong held fitted image")
	}
	record := blocks[1].(*core.UserAttachment)
	if record.SizeBytes != uint64(len(upload.Bytes)) || record.SHA256 != upload.SHA256 {
		t.Fatal("attachment lost original")
	}
	broken, held, err := store.UploadBlocks(t.Context(), testUpload(t, "shot.png", "image/png", brokenPNG), blobs)
	if err != nil || len(broken) != 1 {
		t.Fatalf("broken upload: %v %v", broken, err)
	}
	if _, ok := broken[0].(*core.UserAttachment); !ok {
		t.Fatal("broken image has a native medium")
	}
	_, missing = store.NewModelView(0, []core.Block{&core.UserBlock{Content: blocks}}, held)
	if len(missing) != 1 {
		t.Fatal("broken image held bytes")
	}
}
