package backend_test

import (
	"net/http"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/webapi"
)

// TestDraftReachesEverySessionAndKeepsReplacedVersion checks that draft updates reach all sessions and
// preserve replaced versions.
// One backend and two page sessions; blob and draft storage are local, no model.
func TestDraftReachesEverySessionAndKeepsReplacedVersion(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	backend, laptop, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	phone, err := backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &laptop, conversationFirst)
	path := "/api/conversations/" + conversationFirst + "/draft"
	read := func(s *backendtest.Session) webapi.ConversationDraft {
		return conversationDecode(
			t,
			conversationRequest(ctx, t, backend, s, "GET", path, "", 200),
			webapi.DecodeDraftAnswer,
		).Draft
	}
	conversationEqual(t, read(&phone), webapi.EmptyConversationDraft())
	page, err := backend.Sync(ctx, t, &phone)
	wireMust(t, err)
	snapshot, err := page.Snapshot(ctx)
	wireMust(t, err)
	conversationEqual(t, snapshot.Conversations[0].DraftRevision, uint64(0))
	until := func(revision uint64) {
		_, err := page.Until(ctx, func(e webapi.SyncEvent) bool {
			c, ok := e.(*webapi.SyncEventConversation)
			return ok && c.Conversation.ID == conversationFirst && c.Conversation.DraftRevision == revision
		})
		wireMust(t, err)
	}
	notes := "\n\n  first line of the notes\r\nsecond line"
	response, err := backend.Response(
		ctx,
		"POST",
		"/api/attachments?name=notes.txt",
		&laptop,
		http.Header{"Content-Type": {"text/plain"}},
		strings.NewReader(notes),
	)
	wireMust(t, err)
	uploaded, err := backendtest.ReadAnswer(ctx, response)
	wireMust(t, err)
	upload := conversationDecode(t, uploaded, webapi.DecodeAttachmentAnswer).Attachment
	files := []framewire.ClientContent{
		&framewire.UploadContent{Ref: string(upload.ID), FileName: "notes.txt"},
		&framewire.RemoteFileContent{DeviceID: "a-device", Path: "/var/log/app.log"},
	}
	snippet := "first line of the notes\nsecond line"
	stored := []webapi.DraftFile{
		&webapi.DraftFileUpload{
			Ref:       upload.ID,
			FileName:  "notes.txt",
			MediaType: upload.MediaType,
			Sha256:    upload.Sha256,
			Snippet:   &snippet,
		},
		&webapi.DraftFileRemoteFile{DeviceID: "a-device", Path: "/var/log/app.log"},
	}
	text := func(words string) string { return "Fix the login" + words + " ￼ after ￼" }
	save := func(
		s *backendtest.Session,
		base uint64,
		text string,
		files []framewire.ClientContent,
	) webapi.ConversationDraft {
		return conversationDecode(
			t,
			conversationRequest(
				ctx,
				t,
				backend,
				s,
				"PUT",
				path,
				conversationJSON(t, webapi.DraftSave{Base: base, Text: text, Files: files}),
				200,
			),
			webapi.DecodeDraftAnswer,
		).Draft
	}
	act := func(s *backendtest.Session, action string, revision uint64, status int) backendtest.Answer {
		return conversationRequest(
			ctx,
			t,
			backend,
			s,
			"POST",
			path+"/replaced",
			conversationJSON(t, webapi.ReplacedDraftAction{Action: webapi.ReplacedAction(action), Revision: revision}),
			status,
		)
	}
	first := save(&laptop, 0, text(""), files)
	conversationEqual(t, first, webapi.ConversationDraft{Revision: 1, Text: text(""), Files: stored})
	until(1)
	conversationEqual(t, read(&phone), first)
	bytes := conversationRequest(ctx, t, backend, &phone, "GET", "/api/blobs/"+string(upload.Sha256), "", 200)
	conversationEqual(t, string(bytes.Body), notes)
	testedDraft := save(&phone, 1, text(" test"), files)
	conversationEqual(t, testedDraft.Revision, uint64(2))
	conversationEqual(t, testedDraft.Replaced, (*webapi.ReplacedDraft)(nil))
	bug := save(&laptop, 1, text(" bug"), files)
	tested := &webapi.ReplacedDraft{Revision: 2, Text: text(" test"), Files: stored}
	conversationEqual(t, bug.Revision, uint64(3))
	conversationEqual(t, bug.Text, text(" bug"))
	conversationEqual(t, bug.Replaced, tested)
	until(3)
	conversationEqual(t, read(&phone), bug)
	for _, step := range []struct {
		s              *backendtest.Session
		base, revision uint64
		words          string
	}{
		{&laptop, 3, 4, " bug now"}, {&phone, 2, 5, " bug now"}, {&laptop, 4, 6, " bug now, more"},
	} {
		d := save(step.s, step.base, text(step.words), files)
		conversationEqual(t, d.Revision, step.revision)
		conversationEqual(t, d.Replaced, tested)
	}
	restored := conversationDecode(t, act(&phone, "restore", 2, 200), webapi.DecodeDraftAnswer).Draft
	conversationEqual(
		t,
		restored,
		webapi.ConversationDraft{
			Revision: 7,
			Text:     text(" test"),
			Files:    stored,
			Replaced: &webapi.ReplacedDraft{Revision: 6, Text: text(" bug now, more"), Files: stored},
		},
	)
	conversationRefusal(t, act(&laptop, "restore", 2, 409), webapi.ErrorCodeDraftChanged)
	dismissed := conversationDecode(t, act(&laptop, "dismiss", 6, 200), webapi.DecodeDraftAnswer).Draft
	conversationEqual(t, dismissed.Revision, uint64(8))
	conversationEqual(t, dismissed.Text, text(" test"))
	conversationEqual(t, dismissed.Replaced, (*webapi.ReplacedDraft)(nil))
	conversationRefusal(t, act(&laptop, "dismiss", 6, 409), webapi.ErrorCodeDraftChanged)
	restated := save(&phone, 7, text(" test more"), files)
	conversationEqual(t, restated.Revision, uint64(9))
	conversationEqual(t, restated.Replaced, (*webapi.ReplacedDraft)(nil))
	emptied := save(&laptop, 9, "", []framewire.ClientContent{})
	late := save(&phone, 8, text(" late"), files)
	conversationEqual(t, emptied.Revision, uint64(10))
	conversationEqual(t, late.Revision, uint64(11))
	conversationEqual(t, late.Replaced, (*webapi.ReplacedDraft)(nil))
	for _, refusal := range []struct {
		body   string
		status int
		code   webapi.ErrorCode
	}{
		{conversationJSON(t, webapi.DraftSave{Base: 11, Text: "one mark", Files: files}), 400, webapi.ErrorCodeInvalidBody},
		{`{"base":9,"text":"￼","files":[{"type":"text","text":"not a file"}]}`, 400, webapi.ErrorCodeInvalidBody},
		{`{"base":9,"text":"￼","files":[{"type":"upload","ref":"no-such-upload",` +
			`"fileName":"a.txt"}]}`, 404, webapi.ErrorCodeUploadNotFound},
		{
			conversationJSON(t, webapi.DraftSave{
				Base:  9,
				Text:  strings.Repeat("x", 256*1024),
				Files: []framewire.ClientContent{},
			}),
			413,
			webapi.ErrorCodeTooLarge,
		},
	} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &laptop, "PUT", path, refusal.body, refusal.status),
			refusal.code,
		)
	}
	conversationEqual(t, read(&laptop), late)
	conversationRequest(
		ctx,
		t,
		backend,
		&laptop,
		"PATCH",
		"/api/conversations/"+conversationFirst,
		`{"archived":true}`,
		200,
	)
	conversationEqual(t, read(&phone), late)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, backend, &phone, "PUT", path, `{"base":9,"text":"","files":[]}`, 409),
		webapi.ErrorCodeConversationArchived,
	)
	conversationRefusal(t, act(&phone, "dismiss", 9, 409), webapi.ErrorCodeConversationArchived)
}
