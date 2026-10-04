package scenarios_test

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"fmt"
	"image/png"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// TestRepeatedUploadSendsNoObjectBytes uses two local uploads and no model or runner. Object counts pin
// deduplication.
func TestRepeatedUploadSendsNoObjectBytes(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	counts := &blobstest.ObjectCounts{}
	h.Objects = counts
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	png := storetest.PNG(4, 3, 0)
	before := counts.Tally()
	first := filesUpload(ctx, t, b, &s, "shot.png", "image/png", png)
	stored := counts.Tally().Since(before)
	if stored.Puts != 1 || stored.BytesPut != uint64(len(png)) {
		t.Fatalf("first upload: %+v", stored)
	}
	before = counts.Tally()
	again := filesUpload(ctx, t, b, &s, "copy.png", "image/png", png)
	repeated := counts.Tally().Since(before)
	if again.Sha256 != first.Sha256 {
		t.Fatal("same bytes have different hash")
	}
	if repeated.Puts != 0 || repeated.BytesPut != 0 || repeated.Heads != 1 {
		t.Fatalf("repeated upload: %+v", repeated)
	}
	wireMust(t, b.Close(ctx))
}

// filesAttachment retains the filename supplied by the composer beside the server DTO.
type filesAttachment struct {
	webapiproto.AttachmentDTO
	// Name retains the composer's filename because the server DTO does not carry it.
	Name string
}

// filesUpload sends one attachment using the page's raw upload route.
func filesUpload(
	ctx context.Context,
	t *testing.T,
	b *backendtest.TestBackend,
	s *backendtest.Session,
	name, media string,
	data []byte,
) filesAttachment {
	t.Helper()
	response, err := b.Response(
		ctx,
		"POST",
		"/api/attachments?name="+url.QueryEscape(name),
		s,
		http.Header{"Content-Type": {media}},
		bytes.NewReader(data),
	)
	wireMust(t, err)
	a, err := backendtest.ReadAnswer(ctx, response)
	wireMust(t, err)
	filesStatus(t, a, 201)
	return filesAttachment{
		AttachmentDTO: conversationDecode(t, a, webapiproto.DecodeAttachmentAnswer).Attachment,
		Name:          name,
	}
}

// filesUploadMessage names attachments in the same order as the page's composer.
func filesUploadMessage(id, text string, uploads ...filesAttachment) *conversationproto.SendFrame {
	frame := backendtest.ConversationText(id, text)
	for _, upload := range uploads {
		frame.Content = append(
			frame.Content,
			&conversationproto.UploadContent{Ref: string(upload.ID), FileName: upload.Name},
		)
	}
	return frame
}

// filesCloseTree detaches the page and waits for the durable tree disposal acknowledgement.
func filesCloseTree(ctx context.Context, t *testing.T, s *backendtest.ConversationSocket) {
	t.Helper()
	wireMust(t, s.Send(ctx, &conversationproto.CloseFrame{}))
	_, err := s.Until(ctx, func(f conversationproto.ServerFrame) bool {
		_, ok := f.(*conversationproto.ClosedFrame)
		return ok
	})
	wireMust(t, err)
}

// TestUploadReopeningTwoPagesAndSyncWritesNoBlob uses a local vendor and runner to restore two image references
// without writing objects.
func TestUploadReopeningTwoPagesAndSyncWritesNoBlob(t *testing.T) {
	t.Parallel()
	counts := &blobstest.ObjectCounts{}
	w := filesWorking(t, "", func(h *backendtest.Harness) {
		h.Objects = counts
	})
	shots := []filesAttachment{
		filesUpload(w.ctx, t, w.backend, &w.session, "a.png", "image/png", storetest.PNG(4, 3, 1)),
		filesUpload(w.ctx, t, w.backend, &w.session, "b.png", "image/png", storetest.PNG(4, 3, 2)),
	}
	w.vendor.Respond(conversationAnswer(t, []string{"Two shots."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, filesUploadMessage("m1", "Look", shots...)))
	_, err := w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	filesCloseTree(w.ctx, t, w.socket)
	before := counts.Tally()
	_, err = w.socket.Open(w.ctx)
	wireMust(t, err)
	second := conversationOpen(w.ctx, t, w.backend, &w.session, filesConversation)
	synced, err := second.Live(w.ctx)
	wireMust(t, err)
	after := counts.Tally().Since(before)
	conversationEqual(t, []uint64{after.Puts, after.BytesPut, after.Heads}, []uint64{0, 0, 0})
	user, ok := synced[0].(*types.UserBlock)
	if !ok {
		t.Fatal("no user block")
	}
	images := 0
	for _, part := range user.Content {
		if image, ok := part.(*types.UserImage); ok {
			if _, ok := image.Source.(*types.MediaSourceRef); ok {
				images++
			}
		}
	}
	conversationEqual(t, images, 2)
	wireMust(t, second.Close(w.ctx))
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestUploadRestoreReadsReplayBlobsOnceAfterCompaction reads nine replayed images once with bounded concurrency
// after a compaction.
func TestUploadRestoreReadsReplayBlobsOnceAfterCompaction(t *testing.T) {
	t.Parallel()
	counts := &blobstest.ObjectCounts{}
	w := filesWorking(t, "", func(h *backendtest.Harness) {
		h.Objects = counts
	})
	old := filesUpload(w.ctx, t, w.backend, &w.session, "old.png", "image/png", storetest.PNG(4, 3, 0))
	var shots []filesAttachment
	for i := 1; i <= 9; i++ {
		shots = append(
			shots,
			filesUpload(w.ctx, t, w.backend, &w.session, "shot.png", "image/png", storetest.PNG(4, 3, byte(i))),
		)
	}
	w.vendor.Respond(conversationAnswer(t, []string{"An old shot."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, filesUploadMessage("m1", "Look", old)))
	_, err := w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	w.vendor.Respond(
		providertest.MockResponse{
			Status: 400,
			Chunks: [][]byte{[]byte(`{"type":"error","error":{"type":"invalid_request_error","message":"try later"}}`)},
		},
	)
	wireMust(t, w.socket.Send(w.ctx, filesUploadMessage("m2", "Look at these", shots...)))
	_, err = w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	w.vendor.Respond(conversationAnswer(t, []string{"The user showed an old shot."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, &conversationproto.CompactFrame{}))
	_, err = w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	filesCloseTree(w.ctx, t, w.socket)
	before := counts.Tally()
	_, err = w.socket.Open(w.ctx)
	wireMust(t, err)
	conversationEqual(t, counts.Tally().Since(before).Gets, uint64(0))
	w.vendor.Respond(conversationToolUse(t, "toolu_1", "no_such_tool", `{}`))
	w.vendor.Respond(conversationAnswer(t, []string{"Still nine."}, 1, 1))
	requestCount := len(w.vendor.Requests())
	wireMust(t, w.socket.Send(w.ctx, backendtest.ConversationText("m3", "And now?")))
	w.vendor.Received(w.ctx, requestCount+1)
	first := counts.Tally().Since(before)
	_, err = w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	both := counts.Tally().Since(before)
	conversationEqual(t, []uint64{first.Gets, both.Gets}, []uint64{9, 9})
	if both.MostGetsAtOnce <= 1 || both.MostGetsAtOnce > 8 {
		t.Fatalf("read concurrency: %+v", both)
	}
	for _, request := range w.vendor.Requests()[requestCount:] {
		messages := filesModelField(t, request.Body, "messages")
		for i := 1; i <= 9; i++ {
			filesContains(t, messages, base64.StdEncoding.EncodeToString(storetest.PNG(4, 3, byte(i))))
		}
		if strings.Contains(messages, base64.StdEncoding.EncodeToString(storetest.PNG(4, 3, 0))) {
			t.Fatal("old image replayed")
		}
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestUploadAndToolFitWideImageButKeepHostOriginal fits a 2400px upload and command output identically; Host
// bytes stay whole.
func TestUploadAndToolFitWideImageButKeepHostOriginal(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "")
	wide := storetest.PNG(2400, 10, 1)
	wireMust(t, os.WriteFile(filepath.Join(w.root, "wide.png"), wide, 0o644))
	upload := filesUpload(w.ctx, t, w.backend, &w.session, "wide.png", "image/png", wide)
	w.vendor.Respond(conversationShell(t, "toolu_1", "cat wide.png", 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"Both are wide."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, filesUploadMessage("m1", "Look", upload)))
	_, err := w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	conversationEqual(
		t,
		[]byte(filesRead(t, filepath.Join(w.paired.Runner.Home(), ".demi/attachments", filesConversation, "wide.png"))),
		wide,
	)
	requests := w.vendor.Requests()
	firstImage := func(body []byte) string {
		t.Helper()
		messages := filesModelField(t, body, "messages")
		marker := `"data":"`
		start := strings.Index(messages, marker)
		if start < 0 {
			t.Fatalf("no image: %s", body)
		}
		return strings.SplitN(messages[start+len(marker):], `"`, 2)[0]
	}
	fitted := firstImage(requests[0].Body)
	data, err := base64.StdEncoding.DecodeString(fitted)
	wireMust(t, err)
	config, err := png.DecodeConfig(bytes.NewReader(data))
	wireMust(t, err)
	conversationEqual(t, []int{config.Width, config.Height}, []int{2000, 8})
	if strings.Contains(filesModelField(t, requests[0].Body, "messages"), base64.StdEncoding.EncodeToString(wide)) {
		t.Fatal("original image sent")
	}
	conversationEqual(t, firstImage(requests[1].Body), fitted)
	conversationEqual(t, strings.Count(filesModelField(t, requests[1].Body, "messages"), fitted), 2)
	filesContains(
		t,
		filesModelField(t, requests[1].Body, "messages"),
		"fitted to what every model accepts; to keep the original, save it: demi shell output ",
	)
	user := conversationTranscript(w.ctx, t, w.backend, &w.session, filesConversation).Blocks[0].(*types.UserBlock)
	image := user.Content[1].(*types.UserImage).Source.(*types.MediaSourceRef)
	if image.Ref == upload.Sha256 {
		t.Fatal("original hash in fitted image")
	}
	served := conversationRequest(w.ctx, t, w.backend, &w.session, "GET", "/api/blobs/"+string(image.Ref), "", 200)
	conversationEqual(t, served.Body, data)
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestUploadUnstorableToolMediaBecomesGoneAndTurnContinues fails a local object write to replace tool media with
// a reason and preserve the turn.
func TestUploadUnstorableToolMediaBecomesGoneAndTurnContinues(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "")
	png := storetest.PNG(4, 3, 0)
	wireMust(t, os.WriteFile(filepath.Join(w.root, "shot.png"), png, 0o644))
	blobs := filepath.Join(w.harness.DataDir(), "blobs")
	wireMust(t, os.MkdirAll(blobs, 0o755))
	wireMust(t, os.WriteFile(filepath.Join(blobs, string(w.session.User.ID)), nil, 0o644))
	w.vendor.Respond(conversationShell(t, "toolu_1", "cat shot.png", 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"No picture."}, 1, 1))
	frames, err := w.socket.Chat(w.ctx, "m1", "Show me the picture")
	wireMust(t, err)
	var gone []*types.ToolGone
	for _, f := range frames {
		if p, ok := f.(*conversationproto.TranscriptPatchFrame); ok {
			for _, patch := range p.Patches {
				var block types.Block
				switch v := patch.(type) {
				case *conversationproto.AppendTextPatch, *conversationproto.ReplacePatch:
					// Only incremental tool-call changes can announce a gone result.
				case *conversationproto.AddPatch:
					block = v.Value
				case *conversationproto.ReplaceBlockPatch:
					block = v.Value
				}
				if call, ok := block.(*types.ToolCallBlock); ok {
					for _, part := range call.Output {
						if v, ok := part.(*types.ToolGone); ok {
							gone = append(gone, v)
						}
					}
				}
			}
		}
	}
	conversationEqual(t, len(gone), 1)
	conversationEqual(t, string(gone[0].Kind), "image")
	conversationEqual(t, gone[0].MediaType, "image/png")
	cause, ok := gone[0].Cause.(*types.NotStored)
	if !ok {
		t.Fatalf("cause: %+v", gone[0].Cause)
	}
	encoded := conversationJSON(t, frames)
	filesContains(t, encoded, "No picture.")
	b64 := base64.StdEncoding.EncodeToString(png)
	if strings.Contains(encoded, b64) {
		t.Fatal("frames carry picture bytes")
	}
	continued := filesModelField(t, w.vendor.Requests()[1].Body, "messages")
	filesContains(t, continued, conversationJSON(t, "[image not stored: "+cause.Error+"]"))
	if strings.Contains(continued, b64) {
		t.Fatal("request carries picture bytes")
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestUploadReachesModelHostAndPageByReference uses a real runner and local vendor to carry uploads, a steer and
// an image-preserving edit.
func TestUploadReachesModelHostAndPageByReference(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "")
	wireMust(t, w.harness.AddUser(w.ctx, "ana@example.test", "ana-pass-1", webapiproto.RoleUser))
	png := storetest.PNG(4, 3, 0)
	image := filesUpload(w.ctx, t, w.backend, &w.session, "shot.png", "application/octet-stream", png)
	ref := fmt.Sprintf("%x", sha256.Sum256(png))
	conversationEqual(t, image.MediaType, "image/png")
	conversationEqual(t, image.SizeBytes, uint64(len(png)))
	conversationEqual(t, string(image.Sha256), ref)
	conversationEqual(t, image.Snippet, (*string)(nil))
	notes := filesUpload(
		w.ctx,
		t,
		w.backend,
		&w.session,
		"notes.log",
		"application/octet-stream",
		[]byte("\r\n  first\r\nsecond"),
	)
	conversationEqual(t, notes.MediaType, "application/octet-stream")
	if notes.Snippet == nil || *notes.Snippet != "first\nsecond" {
		t.Fatalf("snippet: %+v", notes.Snippet)
	}
	ana, err := w.backend.Login(w.ctx, "ana@example.test", "ana-pass-1")
	wireMust(t, err)
	hers := filesUpload(w.ctx, t, w.backend, &ana, "hers.txt", "text/plain", []byte("not yours"))
	for _, r := range []struct {
		query, media string
		data         []byte
		status       int
		code         webapiproto.ErrorCode
	}{
		{
			"",
			"image/png",
			png,
			400,
			webapiproto.ErrorCodeInvalidQuery,
		},
		{
			"?name=a.png",
			"",
			png,
			400,
			webapiproto.ErrorCodeInvalidBody,
		},
		{
			"?name=a.png",
			"multipart/form-data; boundary=x",
			png,
			400,
			webapiproto.ErrorCodeInvalidBody,
		},
		{
			"?name=a.png",
			"image/png",
			nil,
			400,
			webapiproto.ErrorCodeInvalidBody,
		},
		{
			"?name=a.bin",
			"application/octet-stream",
			make([]byte, 25*1024*1024+1),
			413,
			webapiproto.ErrorCodeTooLarge,
		},
	} {
		headers := http.Header{}
		if r.media != "" {
			headers.Set("Content-Type", r.media)
		}
		response, err := w.backend.Response(
			w.ctx,
			"POST",
			"/api/attachments"+r.query,
			&w.session,
			headers,
			bytes.NewReader(r.data),
		)
		wireMust(t, err)
		answer, err := backendtest.ReadAnswer(w.ctx, response)
		wireMust(t, err)
		filesRefusal(t, answer, r.status, r.code)
	}
	w.vendor.Respond(conversationAnswer(t, []string{"Seen."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, filesUploadMessage("m1", "Look", image, notes, hers)))
	frames, err := w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	directory := filepath.Join(w.paired.Runner.Home(), ".demi/attachments", filesConversation)
	conversationEqual(t, []byte(filesRead(t, filepath.Join(directory, "shot.png"))), png)
	conversationEqual(t, filesRead(t, filepath.Join(directory, "notes.log")), "\r\n  first\r\nsecond")
	if _, err := os.Stat(filepath.Join(directory, "hers.txt")); !os.IsNotExist(err) {
		t.Fatalf("foreign attachment: %v", err)
	}
	b64 := base64.StdEncoding.EncodeToString(png)
	sent := filesModelField(t, w.vendor.Requests()[0].Body, "messages")
	filesContains(
		t,
		sent,
		b64,
		directory+"/shot.png",
		directory+"/notes.log",
		"[attachment "+string(hers.ID)+" is not available]",
	)
	encoded := conversationJSON(t, frames)
	filesContains(t, encoded, ref)
	if strings.Contains(encoded, b64) {
		t.Fatal("page contains inline image")
	}
	blocks := conversationTranscript(w.ctx, t, w.backend, &w.session, filesConversation).Blocks
	user, ok := blocks[0].(*types.UserBlock)
	if !ok {
		t.Fatalf("first block: %T", blocks[0])
	}
	conversationEqual(
		t,
		conversationKinds(t, user.Content),
		[]string{"text", "image", "attachment", "attachment", "text"},
	)
	picture, ok := user.Content[1].(*types.UserImage)
	if !ok {
		t.Fatal("no image")
	}
	source, ok := picture.Source.(*types.MediaSourceRef)
	if !ok {
		t.Fatalf("source: %T", picture.Source)
	}
	conversationEqual(t, string(source.Ref), ref)
	served := conversationRequest(w.ctx, t, w.backend, &w.session, "GET", "/api/blobs/"+ref, "", 200)
	conversationEqual(t, served.Body, png)
	w.vendor.Respond(conversationAnswer(t, []string{"Again."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, filesUploadMessage("m2", "Once more", image)))
	_, err = w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	conversationEqual(t, []byte(filesRead(t, filepath.Join(directory, "shot-2.png"))), png)
	w.vendor.Respond(conversationShell(t, "wait", "read go", 60000))
	w.vendor.Respond(conversationAnswer(t, []string{"Steered."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, backendtest.ConversationText("m3", "Wait for the file")))
	// Page output identifies the waiting job; input is an event gate instead of a polled file.
	waiting, err := w.socket.Until(w.ctx, func(f conversationproto.ServerFrame) bool {
		v, ok := f.(*conversationproto.ShellOutputFrame)
		return ok && v.Status.Command().ToolUseID == "wait"
	})
	wireMust(t, err)
	command := waiting[len(waiting)-1].(*conversationproto.ShellOutputFrame).Status.Command().CommandID
	wireMust(
		t,
		w.socket.Send(
			w.ctx,
			&conversationproto.SteerFrame{
				SteerID: "s1",
				Content: filesUploadMessage("unused", "And this one", image).Content,
			},
		),
	)
	steered, err := w.socket.Until(w.ctx, func(f conversationproto.ServerFrame) bool {
		_, ok := f.(*conversationproto.SteerResultFrame)
		return ok
	})
	wireMust(t, err)
	last := steered[len(steered)-1].(*conversationproto.SteerResultFrame)
	if _, ok := last.Outcome.(*conversationproto.AcceptedSteer); !ok {
		t.Fatal("steer not accepted")
	}
	wireMust(t, w.socket.Send(w.ctx, &conversationproto.ShellWriteFrame{CommandID: command, Stdin: "go\n"}))
	_, err = w.socket.Until(w.ctx, conversationIdle)
	wireMust(t, err)
	conversationEqual(t, []byte(filesRead(t, filepath.Join(directory, "shot-3.png"))), png)
	filesContains(
		t,
		filesModelField(t, w.vendor.Requests()[3].Body, "messages"),
		"And this one",
		b64,
		directory+"/shot-3.png",
	)
	wireMust(t, w.socket.Send(w.ctx, &conversationproto.SyncTranscriptFrame{}))
	synced, err := w.socket.Until(w.ctx, func(f conversationproto.ServerFrame) bool {
		_, ok := f.(*conversationproto.TranscriptResetFrame)
		return ok
	})
	wireMust(t, err)
	version := synced[len(synced)-1].(*conversationproto.TranscriptResetFrame).Version
	request := conversationproto.EditRequest{
		OperationID:   "keep-1",
		TargetBlockID: user.ID(),
		Version:       version,
		Content: []conversationproto.ClientContent{
			&conversationproto.TextContent{Text: "Look again"},
			&conversationproto.MediaContent{
				Media: &conversationproto.MediaImageRef{Ref: source.Ref, MediaType: source.MediaType},
			},
		},
	}
	w.vendor.Respond(conversationAnswer(t, []string{"The same shot."}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, &conversationproto.EditAndSendFrame{Request: request}))
	edited, err := w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	accepted := false
	for _, f := range edited {
		if e, ok := f.(*conversationproto.EditResultFrame); ok {
			_, accepted = e.Outcome.(*conversationproto.AcceptedEdit)
		}
	}
	if !accepted {
		t.Fatal("edit not accepted")
	}
	requests := w.vendor.Requests()
	conversationEqual(t, len(requests), 5)
	filesContains(t, filesModelField(t, requests[4].Body, "messages"), "Look again", b64)
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}
