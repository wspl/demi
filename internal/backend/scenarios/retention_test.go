package scenarios_test

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/fsnotify/fsnotify"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/plugin/skills"
	"github.com/wspl/demi/internal/plugin/skills/skillstest"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// TestRetentionRunsFirstPassAfterServing restarts over local files, then waits on the deletion event, never
// elapsed time.
func TestRetentionRunsFirstPassAfterServing(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	counts := &blobstest.ObjectCounts{}
	h.Objects = counts
	h.Clock.FollowSystem()
	h.Config.Lifecycle.RetentionInterval = 24 * time.Hour
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	wireMust(t, b.Close(ctx))
	name := filesBlob(t, h, string(s.User.ID), storetest.PNG(2, 2, 4))
	path := filepath.Join(h.DataDir(), "blobs", string(s.User.ID), name)
	now, err := time.Parse(time.RFC3339Nano, string(h.Clock.Now()))
	wireMust(t, err)
	written := now.Add(-25 * time.Hour)
	wireMust(t, os.Chtimes(path, written, written))
	watcher, err := fsnotify.NewWatcher()
	wireMust(t, err)
	defer func() { wireMust(t, watcher.Close()) }()
	wireMust(t, watcher.Add(filepath.Dir(path)))
	b, err = h.Start(ctx, t)
	wireMust(t, err)
	for {
		_, err := os.Stat(path)
		if errors.Is(err, os.ErrNotExist) {
			break
		}
		wireMust(t, err)
		select {
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		case err := <-watcher.Errors:
			wireMust(t, err)
		case <-watcher.Events:
		}
	}
	if got := counts.Tally().Deletes; got != 1 {
		t.Fatalf("deletes = %d, want 1", got)
	}
	wireMust(t, b.Close(ctx))
}

// filesResultMedia observes retained or retired images in the cold transcript.
func filesResultMedia(t *testing.T, w filesWork, id string) []string {
	t.Helper()
	var media []string
	for _, block := range conversationTranscript(w.ctx, t, w.backend, &w.session, filesConversation).Blocks {
		if call, ok := block.(*types.ToolCallBlock); ok && call.ToolUseID == id {
			for _, part := range call.Output {
				switch p := part.(type) {
				case *types.ToolText, *types.ToolVideo:
					// This observation lists only images.
				case *types.ToolImage:
					if ref, ok := p.Source.(*types.ToolMediaRef); ok {
						media = append(media, string(ref.Ref))
					}
				case *types.ToolGone:
					if cause, ok := p.Cause.(*types.Retired); ok && string(p.Kind) == "image" {
						media = append(media, p.MediaType+" retired on "+string(cause.At)[:10])
					}
				}
			}
			return media
		}
	}
	t.Fatalf("no call %s", id)
	return nil
}

// TestRetentionIdleConversationRetiresToolImagesAndReplaysText restores a real image-producing job after the
// manual retention clock advances.
func TestRetentionIdleConversationRetiresToolImagesAndReplaysText(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "", func(h *backendtest.Harness) { h.Clock.FollowSystem() })
	png := storetest.PNG(2, 2, 1)
	wireMust(t, os.WriteFile(filepath.Join(w.root, "shot.png"), png, 0o644))
	w.vendor.Respond(conversationShell(t, "toolu_1", "cat shot.png", 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"Seen."}, 1, 1))
	_, err := w.socket.Chat(w.ctx, "m1", "Show me the shot")
	wireMust(t, err)
	filesCloseTree(w.ctx, t, w.socket)
	wireMust(t, w.harness.Clock.Advance(31*24*time.Hour))
	w.session, err = w.backend.Login(w.ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	wireMust(t, backendtest.RunRetention(w.ctx, w.backend.Backend, w.session.User.ID))
	day := string(w.harness.Clock.Now())[:10]
	conversationEqual(t, filesResultMedia(t, w, "toolu_1"), []string{"image/png retired on " + day})
	_, err = w.socket.Open(w.ctx)
	wireMust(t, err)
	w.vendor.Respond(conversationAnswer(t, []string{"Still here."}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m2", "Anything new?")
	wireMust(t, err)
	requests := w.vendor.Requests()
	sent := filesModelField(t, requests[len(requests)-1].Body, "messages")
	filesContains(
		t,
		sent,
		"[image:image/png, removed on "+day+": a tool result's images and videos are kept for 30 days]",
	)
	if strings.Contains(sent, base64.StdEncoding.EncodeToString(png)) {
		t.Fatal("retired image replayed")
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestRetentionRemovesCommandOutputThirtyDaysAfterEnd uses three jobs to read saved stdout before and after a
// manual 31-day retention advance.
func TestRetentionRemovesCommandOutputThirtyDaysAfterEnd(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "", func(h *backendtest.Harness) { h.Clock.FollowSystem() })
	w.vendor.Respond(conversationShell(t, "toolu_1", "echo kept", 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"Said."}, 1, 1))
	_, err := w.socket.Chat(w.ctx, "m1", "Say it")
	wireMust(t, err)
	command := toolstest.Field(conversationToolResult(t, w.vendor.Requests()[1], "toolu_1"), "commandId")
	read := "demi shell output " + command + " --raw"
	w.vendor.Respond(conversationShell(t, "toolu_2", read, 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"Read."}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m2", "Read it")
	wireMust(t, err)
	requests := w.vendor.Requests()
	filesContains(t, conversationToolResult(t, requests[len(requests)-1], "toolu_2"), "kept")
	wireMust(t, w.harness.Clock.Advance(31*24*time.Hour))
	w.session, err = w.backend.Login(w.ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	wireMust(t, backendtest.RunRetention(w.ctx, w.backend.Backend, w.session.User.ID))
	w.vendor.Respond(conversationShell(t, "toolu_3", read, 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"Gone."}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m3", "Read it again")
	wireMust(t, err)
	requests = w.vendor.Requests()
	filesContains(
		t,
		conversationToolResult(t, requests[len(requests)-1], "toolu_3"),
		"demi shell output: the output of "+command+" was removed on "+string(w.harness.Clock.Now())[:10]+
			", 30 days after the command ended",
	)
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestRetentionSummarizedImagesWaitForPageAndBlobGrace uses compaction plus a manual 31-day advance to test page
// leases and the blob grace period.
func TestRetentionSummarizedImagesWaitForPageAndBlobGrace(t *testing.T) {
	t.Parallel()
	counts := &blobstest.ObjectCounts{}
	w := filesWorking(t, "", func(h *backendtest.Harness) {
		h.Clock.FollowSystem()
		h.Objects = counts
	})
	shot, later, pastedPNG := storetest.PNG(2, 2, 1), storetest.PNG(2, 2, 2), storetest.PNG(2, 2, 3)
	wireMust(t, os.WriteFile(filepath.Join(w.root, "shot.png"), shot, 0o644))
	wireMust(t, os.WriteFile(filepath.Join(w.root, "later.png"), later, 0o644))
	pasted := filesUpload(w.ctx, t, w.backend, &w.session, "pasted.png", "image/png", pastedPNG)
	w.vendor.Respond(conversationShell(t, "toolu_1", "cat shot.png", 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"Seen."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, filesUploadMessage("m1", "Look", pasted)))
	_, err := w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	w.vendor.Respond(conversationAnswer(t, []string{"Read."}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m2", strings.Repeat("a long note ", 2000))
	wireMust(t, err)
	w.vendor.Respond(conversationAnswer(t, []string{"The user showed two images."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, &conversationproto.CompactFrame{}))
	_, err = w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	w.vendor.Respond(conversationShell(t, "toolu_2", "cat later.png", 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"Seen again."}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m3", "And the later one")
	wireMust(t, err)
	shotRef, laterRef := fmt.Sprintf("%x", sha256.Sum256(shot)), fmt.Sprintf("%x", sha256.Sum256(later))
	wireMust(t, w.harness.Clock.Advance(31*24*time.Hour))
	w.session, err = w.backend.Login(w.ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	wireMust(t, backendtest.RunRetention(w.ctx, w.backend.Backend, w.session.User.ID))
	conversationEqual(t, filesResultMedia(t, w, "toolu_1"), []string{shotRef})
	filesCloseTree(w.ctx, t, w.socket)
	wireMust(
		t,
		backendtest.WaitFile(
			w.ctx,
			filepath.Join(w.harness.DataDir(), "conversations", filesConversation+".sqlite"),
			func([]byte) bool {
				media := filesResultMedia(t, w, "toolu_1")
				return len(media) == 1 && media[0] == "image/png retired on "+string(w.harness.Clock.Now())[:10]
			},
		),
	)
	conversationEqual(
		t,
		filesResultMedia(t, w, "toolu_1"),
		[]string{"image/png retired on " + string(w.harness.Clock.Now())[:10]},
	)
	conversationEqual(t, filesResultMedia(t, w, "toolu_2"), []string{laterRef})
	blocks := conversationTranscript(w.ctx, t, w.backend, &w.session, filesConversation).Blocks
	user, ok := blocks[0].(*types.UserBlock)
	if !ok {
		t.Fatalf("first block: %T", blocks[0])
	}
	image := false
	for _, part := range user.Content {
		if _, ok := part.(*types.UserImage); ok {
			image = true
		}
	}
	if !image {
		t.Fatal("user image retired")
	}
	wireMust(t, w.harness.Clock.Advance(25*time.Hour))
	before := counts.Tally()
	wireMust(t, backendtest.RunRetention(w.ctx, w.backend.Backend, w.session.User.ID))
	conversationEqual(t, counts.Tally().Since(before).Deletes, uint64(3))
	root := filepath.Join(w.harness.DataDir(), "blobs", string(w.session.User.ID))
	if _, err := os.Stat(filepath.Join(root, shotRef)); !os.IsNotExist(err) {
		t.Fatalf("retired blob still exists: %v", err)
	}
	for _, ref := range []string{laterRef, string(pasted.Sha256)} {
		_, err := os.Stat(filepath.Join(root, ref))
		wireMust(t, err)
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestRetentionArchiveSucceedsWhenDeviceDiesDuringRelease uses a native fixture to hold release until its real
// runner dies; archive still commits.
func TestRetentionArchiveSucceedsWhenDeviceDiesDuringRelease(t *testing.T) {
	t.Parallel()
	ctx, b, s, paired := conversationStreamDevice(t)
	conversationCreate(ctx, t, b, &s, conversationSecond)
	filesMove(ctx, t, b, &s, conversationSecond, paired, paired.Runner.Home())
	_, code, reason := conversationStreamEnd(
		ctx,
		t,
		conversationStream(ctx, t, b, &s, conversationFirst, "stall_release"),
	)
	conversationEqual(t, code, 1000)
	conversationEqual(t, reason, "completed")
	type result struct {
		answer backendtest.Answer
		err    error
	}
	archived := make(chan result, 1)
	operation, cancel := context.WithCancel(ctx)
	finished := make(chan struct{})
	defer func() {
		cancel()
		<-finished
	}()
	go func() {
		defer close(finished)
		answer, err := b.Patch(operation, "/api/conversations/"+conversationFirst, &s, []byte(`{"archived":true}`))
		archived <- result{answer, err}
	}()
	_, code, reason = conversationStreamEnd(ctx, t, conversationStream(ctx, t, b, &s, conversationSecond, "stalled"))
	conversationEqual(t, code, 1000)
	conversationEqual(t, reason, "completed")
	wireMust(t, paired.Runner.Kill(ctx))
	select {
	case r := <-archived:
		wireMust(t, r.err)
		filesStatus(t, r.answer, 200)
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	wireMust(t, b.Close(ctx))
}

// TestRetentionCollectsOnlyOldOrphansWhenEveryDatabaseReadable uses a real job, skill source, uploads and draft
// to protect referenced blobs during collection.
func TestRetentionCollectsOnlyOldOrphansWhenEveryDatabaseReadable(t *testing.T) {
	t.Parallel()
	counts := &blobstest.ObjectCounts{}
	repos := skillstest.New(t)
	skill := skillstest.SkillMD("name: review\ndescription: Review a change.")
	_, err := repos.Commit(
		t.Context(),
		"acme/tools",
		[]skillstest.File{{Path: "review/SKILL.md", Bytes: []byte(skill)}},
	)
	wireMust(t, err)
	factory, err := repos.Factory()
	wireMust(t, err)
	w := filesWorking(t, "", func(h *backendtest.Harness) {
		h.Clock.FollowSystem()
		h.Objects = counts
		for i, f := range h.Config.Plugins {
			if f.Manifest().ID == "skills" {
				h.Config.Plugins[i] = factory
			}
		}
	})
	page, _ := conversationPage(w.ctx, t, w.backend, &w.session)
	conversationRequest(
		w.ctx,
		t,
		w.backend,
		&w.session,
		"POST",
		"/api/plugins/skills/calls/add_source",
		conversationJSON(t, skills.AddSource{Origin: "acme/tools"}),
		200,
	)
	_, err = page.Until(w.ctx, func(e webapiproto.SyncEvent) bool {
		p, ok := e.(*webapiproto.SyncEventPlugin)
		if !ok || p.Plugin != "skills" {
			return false
		}
		state, err := skills.DecodeSkillsState(p.State)
		wireMust(t, err)
		return len(state.Sources) > 0 && state.Sources[0].Commit != nil
	})
	wireMust(t, err)
	png := func(seed byte) []byte { return storetest.PNG(2, 2, seed) }
	wireMust(t, os.WriteFile(filepath.Join(w.root, "shot.png"), png(1), 0o644))
	w.vendor.Respond(conversationShell(t, "toolu_0", "printf 'noted\\n' > note.txt", 60000))
	w.vendor.Respond(conversationShell(t, "toolu_1", "cat shot.png", 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"Seen."}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m1", "Show me the shot")
	wireMust(t, err)
	filesUpload(w.ctx, t, w.backend, &w.session, "kept.png", "image/png", png(2))
	staged := filesUpload(w.ctx, t, w.backend, &w.session, "staged.png", "image/png", png(3))
	conversationCreate(w.ctx, t, w.backend, &w.session, conversationSecond)
	body := `{"base":0,"text":"\ufffc","files":[{"type":"upload","ref":"` + string(
		staged.ID,
	) + `","fileName":"staged.png"}]}`
	conversationRequest(
		w.ctx,
		t,
		w.backend,
		&w.session,
		"PUT",
		"/api/conversations/"+conversationSecond+"/draft",
		body,
		200,
	)
	wireMust(t, w.harness.Clock.Advance(25*time.Hour))
	now, err := w.harness.Clock.Now().Time()
	wireMust(t, err)
	root := filepath.Join(w.harness.DataDir(), "blobs", string(w.session.User.ID))
	orphan := func(data []byte, age time.Duration) string {
		name := filesBlob(t, w.harness, string(w.session.User.ID), data)
		path := filepath.Join(root, name)
		written := now.Add(-age)
		wireMust(t, os.Chtimes(path, written, written))
		return path
	}
	left := orphan(png(4), 25*time.Hour)
	recent := orphan(png(5), time.Hour)
	kept := []string{recent}
	for _, data := range [][]byte{png(1), png(2), png(3), {}, []byte("noted\n"), []byte(skill)} {
		kept = append(kept, filepath.Join(root, fmt.Sprintf("%x", sha256.Sum256(data))))
	}
	database := filepath.Join(w.harness.DataDir(), "conversations", conversationSecond+".sqlite")
	wireMust(t, os.WriteFile(database, []byte("not a database"), 0o644))
	before := counts.Tally()
	// The intentionally corrupt source reports errors; collection must still delete nothing.
	_ = backendtest.RunRetention(w.ctx, w.backend.Backend, w.session.User.ID)
	conversationEqual(t, counts.Tally().Since(before).Deletes, uint64(0))
	_, err = os.Stat(left)
	wireMust(t, err)
	wireMust(t, os.Remove(database))
	before = counts.Tally()
	wireMust(t, backendtest.RunRetention(w.ctx, w.backend.Backend, w.session.User.ID))
	collected := counts.Tally().Since(before)
	conversationEqual(t, collected.Lists, uint64(1))
	conversationEqual(t, collected.Deletes, uint64(1))
	if _, err := os.Stat(left); !os.IsNotExist(err) {
		t.Fatalf("old orphan remains: %v", err)
	}
	for _, path := range kept {
		_, err := os.Stat(path)
		wireMust(t, err)
	}
	wireMust(t, page.Close(w.ctx))
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}
