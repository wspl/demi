package store_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"reflect"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

func TestMediaHeldOnceUntilReleased(t *testing.T) {
	blobs := storetest.NewMemoryBlobs()
	parts, held := store.PersistResult(t.Context(), []provider.ResultPart{&provider.TextPart{Text: "hello"}, &provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte("image"), MediaType: "image/png"}}, &provider.ResultVideo{Bytes: provider.MediaBytes{Data: []byte("video"), MediaType: "video/webm"}}}, blobs)
	block := &core.ToolCallBlock{Output: parts}
	refs := store.References(block)
	if len(parts) != 3 || len(refs) != 2 {
		t.Fatalf("stored result: %#v, %v", parts, refs)
	}
	view, missing := store.NewModelView(7, []core.Block{block}, held)
	if len(missing) != 0 || view.Start != 7 {
		t.Fatal("held view not ready")
	}
	if !bytes.Equal(view.Held(refs[0]).(*store.HeldBytes).Bytes, []byte("image")) {
		t.Fatal("wrong image bytes")
	}
	held.Hold(refs[0], []byte("replacement"))
	other := store.HeldMedia{}
	other.Hold(refs[0], []byte("replacement"))
	held.Absorb(other)
	held.Retain(map[core.BlobRef]struct{}{refs[0]: {}})
	_, missing = store.NewModelView(0, []core.Block{block}, held)
	if !reflect.DeepEqual(missing, refs[1:]) {
		t.Fatalf("retained media: %v", missing)
	}
	blobs.Forget(refs[1])
	restored, err := store.ReadMedia(t.Context(), blobs, missing)
	if err != nil {
		t.Fatal(err)
	}
	held.Absorb(restored)
	view, missing = store.NewModelView(0, []core.Block{block}, held)
	if len(missing) != 0 {
		t.Fatal("known missing blob not held")
	}
	if _, ok := view.Held(refs[1]).(*store.HeldMissing); !ok {
		t.Fatal("missing blob became bytes")
	}
	if got := view.Held(refs[0]).(*store.HeldBytes).Bytes; string(got) != "image" {
		t.Fatalf("held medium changed form: %q", got)
	}
	if store.MissingText("video") != "[missing video]" {
		t.Fatal("wrong missing text")
	}
	defer func() {
		if got := recover(); got != "the model's view holds something for every medium its blocks reference" {
			t.Fatalf("invariant panic: %v", got)
		}
	}()
	view.Held("outside")
}

type failedBlobs struct{ err error }

func (b failedBlobs) Put(context.Context, core.B64Bytes) (core.BlobRef, error) { return "", b.err }
func (b failedBlobs) Read(context.Context, core.BlobRef) (core.B64Bytes, bool, error) {
	return nil, false, b.err
}

func TestFailedMediaPutBecomesGoneAndReadFails(t *testing.T) {
	failure := errors.New("disk refused")
	parts, held := store.PersistResult(t.Context(), []provider.ResultPart{&provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte("image"), MediaType: "image/png"}}}, failedBlobs{failure})
	gone, ok := parts[0].(*core.ToolGone)
	if !ok || gone.Kind != "image" || gone.MediaType != "image/png" || gone.Cause.(*core.NotStored).Error != "disk refused" {
		t.Fatalf("gone result: %#v", parts)
	}
	if _, missing := store.NewModelView(0, []core.Block{&core.ToolCallBlock{Output: parts}}, held); len(missing) != 0 {
		t.Fatal("failed put left a reference")
	}
	if _, err := store.ReadMedia(t.Context(), failedBlobs{failure}, []core.BlobRef{"blob"}); !errors.Is(err, failure) {
		t.Fatalf("read failure: %v", err)
	}
	upload := testUpload(t, "wide.png", "image/png", storetest.PNG(2400, 10, 1))
	if _, _, err := store.UploadBlocks(t.Context(), upload, failedBlobs{failure}); !errors.Is(err, failure) {
		t.Fatalf("fitted upload store failure: %v", err)
	}
}

func TestRetentionReferencesIncludeMediaThenEditCopies(t *testing.T) {
	files := []core.EditedFile{{Edits: []core.EditSegment{{Copies: &core.EditCopies{Original: "old", Modified: "new"}}, {Copies: nil}}}}
	block := &core.ToolCallBlock{Output: []core.ToolResultContentBlock{&core.ToolImage{Source: &core.ToolMediaRef{Ref: "image"}}, &core.ToolVideo{Source: &core.ToolMediaRef{Ref: "video"}}, &core.ToolText{Text: "plain"}}, View: &core.ShellView{ShellToolView: core.ShellToolView{Files: &files}}}
	want := []store.BlockReference{{Blob: "image", Holder: store.ToolResult}, {Blob: "video", Holder: store.ToolResult}, {Blob: "old", Holder: store.EditCopy}, {Blob: "new", Holder: store.EditCopy}}
	if got := store.BlockReferences(block); !reflect.DeepEqual(got, want) {
		t.Fatalf("retention refs: %v", got)
	}
	content := []core.UserContentBlock{&core.UserImage{Source: &core.MediaURL{URL: "https://example.test/image"}}, &core.UserDocument{Source: &core.DocumentRef{Ref: "pdf"}}, &core.UserVideo{Source: &core.MediaSourceRef{Ref: "clip"}}}
	if got := store.ContentReferences(content); !reflect.DeepEqual(got, []core.BlobRef{"pdf", "clip"}) {
		t.Fatalf("content refs: %v", got)
	}
	if got := store.BlockReferences(&core.SteerBlock{Content: content}); !reflect.DeepEqual(got, []store.BlockReference{{Blob: "pdf", Holder: store.Message}, {Blob: "clip", Holder: store.Message}}) {
		t.Fatalf("steer refs: %v", got)
	}
}

type heldReads struct {
	mu                    sync.Mutex
	active, peak, started int
	entered               chan struct{}
	release               chan struct{}
}

func (*heldReads) Put(context.Context, core.B64Bytes) (core.BlobRef, error) {
	return "", errors.New("unused put")
}
func (b *heldReads) Read(ctx context.Context, _ core.BlobRef) (core.B64Bytes, bool, error) {
	b.mu.Lock()
	b.active++
	b.started++
	b.peak = max(b.peak, b.active)
	eighth := b.started == 8
	b.mu.Unlock()
	if eighth {
		close(b.entered)
	}
	defer func() { b.mu.Lock(); b.active--; b.mu.Unlock() }()
	select {
	case <-ctx.Done():
		return nil, false, ctx.Err()
	case <-b.release:
		return []byte("medium"), true, nil
	}
}
func TestMediaReadsAreBoundedAndJoinedOnCancellation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		blobs := &heldReads{entered: make(chan struct{}), release: make(chan struct{})}
		ctx, cancel := context.WithCancel(t.Context())
		var workers sync.WaitGroup
		defer func() { cancel(); workers.Wait() }()
		refs := make([]core.BlobRef, 12)
		for i := range refs {
			refs[i] = core.BlobRef(fmt.Sprint(i))
		}
		done := make(chan error, 1)
		workers.Go(func() { _, err := store.ReadMedia(ctx, blobs, refs); done <- err })
		<-blobs.entered
		synctest.Wait()
		blobs.mu.Lock()
		started, peak := blobs.started, blobs.peak
		blobs.mu.Unlock()
		if started != 8 || peak != 8 {
			t.Fatalf("reads exceeded bound: started=%d peak=%d", started, peak)
		}
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("canceled read: %v", err)
		}
		blobs.mu.Lock()
		active := blobs.active
		blobs.mu.Unlock()
		if active != 0 {
			t.Fatal("read workers outlived caller")
		}
	})
}

func TestModelViewOwnsItsTranscriptSnapshot(t *testing.T) {
	text := &core.UserText{Text: "original"}
	selection := storetest.ModelReading("stub", "model", []core.FileExtension{"png"})
	user := &core.UserBlock{BlockID: "u1", Selection: selection, Content: []core.UserContentBlock{text}}
	view, missing := store.NewModelView(0, []core.Block{user}, store.HeldMedia{})
	if len(missing) != 0 {
		t.Fatal("text unexpectedly needs media")
	}
	text.Text = "changed"
	(*selection.Model.AcceptedExtensions)[0] = "pdf"
	snapshot := view.Blocks[0].(*core.UserBlock)
	if snapshot.Content[0].(*core.UserText).Text != "original" || (*snapshot.Selection.Model.AcceptedExtensions)[0] != "png" {
		t.Fatal("view changed with live transcript")
	}
}
