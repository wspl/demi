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
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

func TestMediaHeldOnceUntilReleased(t *testing.T) {
	blobs := storetest.NewMemoryBlobs()
	parts, held := store.PersistResult(
		t.Context(),
		[]provider.ResultPart{
			&provider.TextPart{Text: "hello"},
			&provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte("image"), MediaType: "image/png"}},
			&provider.ResultVideo{Bytes: provider.MediaBytes{Data: []byte("video"), MediaType: "video/webm"}},
		},
		blobs,
	)
	block := &types.ToolCallBlock{Output: parts}
	refs := store.References(block)
	if len(parts) != 3 || len(refs) != 2 {
		t.Fatalf("stored result: %#v, %v", parts, refs)
	}
	view, missing := store.NewModelView(7, []types.Block{block}, held)
	if len(missing) != 0 || view.Start != 7 {
		t.Fatal("held view not ready")
	}
	if data, _ := view.Held(refs[0]); !bytes.Equal(data, []byte("image")) {
		t.Fatal("wrong image bytes")
	}
	held.Hold(refs[0], []byte("replacement"))
	other := store.HeldMedia{}
	other.Hold(refs[0], []byte("replacement"))
	held.Absorb(other)
	held.Retain(map[types.BlobRef]struct{}{refs[0]: {}})
	_, missing = store.NewModelView(0, []types.Block{block}, held)
	if !reflect.DeepEqual(missing, refs[1:]) {
		t.Fatalf("retained media: %v", missing)
	}
	blobs.Forget(refs[1])
	restored, err := store.ReadMedia(t.Context(), blobs, missing)
	if err != nil {
		t.Fatal(err)
	}
	held.Absorb(restored)
	view, missing = store.NewModelView(0, []types.Block{block}, held)
	if len(missing) != 0 {
		t.Fatal("known missing blob not held")
	}
	if _, found := view.Held(refs[1]); found {
		t.Fatal("missing blob became bytes")
	}
	if got, _ := view.Held(refs[0]); string(got) != "image" {
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

func (b failedBlobs) Put(context.Context, types.B64Bytes) (types.BlobRef, error) {
	return "", b.err
}

func (b failedBlobs) Read(context.Context, types.BlobRef) (types.B64Bytes, bool, error) {
	return nil, false, b.err
}

func TestFailedMediaPutBecomesGoneAndReadFails(t *testing.T) {
	failure := errors.New("disk refused")
	parts, held := store.PersistResult(
		t.Context(),
		[]provider.ResultPart{
			&provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte("image"), MediaType: "image/png"}},
		},
		failedBlobs{failure},
	)
	gone, ok := parts[0].(*types.ToolGone)
	if !ok || gone.Kind != "image" || gone.MediaType != "image/png" ||
		gone.Cause.(*types.NotStored).Error != "disk refused" {
		t.Fatalf("gone result: %#v", parts)
	}
	if _, missing := store.NewModelView(
		0,
		[]types.Block{&types.ToolCallBlock{Output: parts}},
		held,
	); len(
		missing,
	) != 0 {
		t.Fatal("failed put left a reference")
	}
	if _, err := store.ReadMedia(t.Context(), failedBlobs{failure}, []types.BlobRef{"blob"}); !errors.Is(err, failure) {
		t.Fatalf("read failure: %v", err)
	}
	upload := testUpload(t, "wide.png", "image/png", storetest.PNG(2400, 10, 1))
	if _, _, err := store.UploadBlocks(t.Context(), upload, failedBlobs{failure}); !errors.Is(err, failure) {
		t.Fatalf("fitted upload store failure: %v", err)
	}
}

func TestRetentionReferencesIncludeMediaThenEditCopies(t *testing.T) {
	files := []types.EditedFile{
		{Edits: []types.EditSegment{{Copies: &types.EditCopies{Original: "old", Modified: "new"}}, {Copies: nil}}},
	}
	block := &types.ToolCallBlock{
		Output: []types.ToolResultContentBlock{
			&types.ToolImage{Source: &types.ToolMediaRef{Ref: "image"}},
			&types.ToolVideo{Source: &types.ToolMediaRef{Ref: "video"}},
			&types.ToolText{Text: "plain"},
		},
		View: &types.ShellView{ShellToolView: types.ShellToolView{Files: &files}},
	}
	want := []store.BlockReference{
		{Blob: "image", Holder: store.ToolResult},
		{Blob: "video", Holder: store.ToolResult},
		{Blob: "old", Holder: store.EditCopy},
		{Blob: "new", Holder: store.EditCopy},
	}
	if got := store.BlockReferences(block); !reflect.DeepEqual(got, want) {
		t.Fatalf("retention refs: %v", got)
	}
	content := []types.UserContentBlock{
		&types.UserImage{Source: &types.MediaURL{URL: "https://example.test/image"}},
		&types.UserDocument{Source: &types.DocumentRef{Ref: "pdf"}},
		&types.UserVideo{Source: &types.MediaSourceRef{Ref: "clip"}},
	}
	if got := store.ContentReferences(content); !reflect.DeepEqual(got, []types.BlobRef{"pdf", "clip"}) {
		t.Fatalf("content refs: %v", got)
	}
	if got := store.BlockReferences(
		&types.SteerBlock{Content: content},
	); !reflect.DeepEqual(
		got,
		[]store.BlockReference{{Blob: "pdf", Holder: store.Message}, {Blob: "clip", Holder: store.Message}},
	) {
		t.Fatalf("steer refs: %v", got)
	}
}

type heldReads struct {
	mu                    sync.Mutex
	active, peak, started int
	entered               chan struct{}
	release               chan struct{}
}

func (*heldReads) Put(context.Context, types.B64Bytes) (types.BlobRef, error) {
	return "", errors.New("unused put")
}

func (b *heldReads) Read(ctx context.Context, _ types.BlobRef) (types.B64Bytes, bool, error) {
	b.mu.Lock()
	b.active++
	b.started++
	b.peak = max(b.peak, b.active)
	eighth := b.started == 8
	b.mu.Unlock()
	if eighth {
		close(b.entered)
	}
	defer func() {
		b.mu.Lock()
		b.active--
		b.mu.Unlock()
	}()
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
		defer func() {
			cancel()
			workers.Wait()
		}()
		refs := make([]types.BlobRef, 12)
		for i := range refs {
			refs[i] = types.BlobRef(fmt.Sprint(i))
		}
		done := make(chan error, 1)
		workers.Go(func() {
			_, err := store.ReadMedia(ctx, blobs, refs)
			done <- err
		})
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
	text := &types.UserText{Text: "original"}
	selection := storetest.ModelReading("stub", "model", []types.FileExtension{"png"})
	user := &types.UserBlock{BlockID: "u1", Selection: selection, Content: []types.UserContentBlock{text}}
	view, missing := store.NewModelView(0, []types.Block{user}, store.HeldMedia{})
	if len(missing) != 0 {
		t.Fatal("text unexpectedly needs media")
	}
	text.Text = "changed"
	(*selection.Model.AcceptedExtensions)[0] = "pdf"
	snapshot := view.Blocks[0].(*types.UserBlock)
	if snapshot.Content[0].(*types.UserText).Text != "original" ||
		(*snapshot.Selection.Model.AcceptedExtensions)[0] != "png" {
		t.Fatal("view changed with live transcript")
	}
}
