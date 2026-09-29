package backendtest_test

import (
	"net/http"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

// draftMark is the mark a draft's text holds where a file's capsule stands.
const draftMark = "￼"

// untilDraft waits until the page's channel brings the conversation's summary
// with the draft revision.
func untilDraft(page *backendtest.SyncChannel, id string, revision float64) {
	page.Until(func(event backendtest.Frame) bool {
		return event["type"] == "conversation" &&
			backendtest.At(event, "conversation.id") == id &&
			backendtest.At(event, "conversation.draftRevision") == revision
	})
}

func readDraft(b *backendtest.Backend, session *backendtest.Session, id string) backendtest.Map {
	return b.Get("/api/conversations/"+id+"/draft", session).Expect(http.StatusOK).At("draft").(map[string]any)
}

func putDraft(b *backendtest.Backend, session *backendtest.Session, id string, base int, text string, files any) *backendtest.Answer {
	return b.Put("/api/conversations/"+id+"/draft", session, backendtest.Map{"base": base, "text": text, "files": files})
}

func saveDraft(b *backendtest.Backend, session *backendtest.Session, id string, base int, text string, files any) backendtest.Map {
	return putDraft(b, session, id, base, text, files).Expect(http.StatusOK).At("draft").(map[string]any)
}

func actOnDraft(b *backendtest.Backend, session *backendtest.Session, id, action string, revision int) *backendtest.Answer {
	return b.Post("/api/conversations/"+id+"/draft/replaced", session, backendtest.Map{"action": action, "revision": revision})
}

// upload stages a file and answers its record.
func upload(t *testing.T, b *backendtest.Backend, session *backendtest.Session, name, mediaType string, content []byte) map[string]any {
	t.Helper()
	sent := b.Do(backendtest.Request{
		Method: http.MethodPost, Path: "/api/attachments?name=" + name, Session: session, Raw: content, ContentType: mediaType,
	}).Expect(http.StatusCreated)
	return sent.At("attachment").(map[string]any)
}

func draftIs(t *testing.T, draft backendtest.Map, revision int, text string, files any, replaced any) {
	t.Helper()
	backendtest.AssertJSON(t, draft, backendtest.Map{"revision": revision, "text": text, "files": files, "replaced": replaced})
}

// Cost: one backend, about a second; a phone's channel and a laptop's requests.
func TestADraftReachesEverySessionAndASaveBuiltOnAnOlderRevisionKeepsWhatItReplaced(t *testing.T) {
	t.Parallel()
	b, laptop := backendtest.New(t).StartSetUp()
	phone := b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	id := createConversation(t, b, laptop)
	draftIs(t, readDraft(b, phone, id), 0, "", []any{}, nil)
	phonePage := b.Sync(phone)
	if revision := backendtest.At(phonePage.Snapshot(), "conversations.0.draftRevision"); revision != 0.0 {
		t.Fatalf("a new conversation's draft is at revision %v", revision)
	}

	// The laptop stages a text file, and a file on a device, and saves the draft
	// that names them.
	notes := []byte("\n\n  first line of the notes\r\nsecond line")
	upload := upload(t, b, laptop, "notes.txt", "text/plain", notes)
	files := []any{
		backendtest.Map{"type": "upload", "ref": upload["id"], "fileName": "notes.txt"},
		backendtest.Map{"type": "remote_file", "deviceId": "a-device", "path": "/var/log/app.log"},
	}
	stored := []any{
		backendtest.Map{
			"type": "upload", "ref": upload["id"], "fileName": "notes.txt", "mediaType": upload["mediaType"],
			"sha256": upload["sha256"], "snippet": "first line of the notes\nsecond line",
		},
		backendtest.Map{"type": "remote_file", "deviceId": "a-device", "path": "/var/log/app.log"},
	}
	text := func(words string) string { return "Fix the login" + words + " " + draftMark + " after " + draftMark }
	first := saveDraft(b, laptop, id, 0, text(""), files)
	draftIs(t, first, 1, text(""), stored, nil)

	// The phone learns of it from its channel and reads it: the text, the file
	// with what its record holds, and the file's bytes.
	untilDraft(phonePage, id, 1)
	backendtest.AssertJSON(t, readDraft(b, phone, id), first)
	bytes := b.Get("/api/blobs/"+upload["sha256"].(string), phone)
	if string(bytes.Body) != string(notes) {
		t.Fatalf("the upload reads back as %q", bytes.Body)
	}

	// Both type on revision 1 at once. The phone's save lands first; the
	// laptop's, built on revision 1 as well, wins and keeps the phone's.
	test := saveDraft(b, phone, id, 1, text(" test"), files)
	if test["revision"] != 2.0 || test["replaced"] != nil {
		t.Fatalf("the phone's save is %v", test)
	}
	bug := saveDraft(b, laptop, id, 1, text(" bug"), files)
	tested := backendtest.Map{"revision": 2, "text": text(" test"), "files": stored}
	draftIs(t, bug, 3, text(" bug"), stored, tested)
	untilDraft(phonePage, id, 3)
	backendtest.AssertJSON(t, readDraft(b, phone, id), bug)
	// A save on the current revision keeps the replaced version, and so does one
	// built on an older revision that saves what the draft holds.
	draftIs(t, saveDraft(b, laptop, id, 3, text(" bug now"), files), 4, text(" bug now"), stored, tested)
	draftIs(t, saveDraft(b, phone, id, 2, text(" bug now"), files), 5, text(" bug now"), stored, tested)
	// That save wrote no new text: one built on the revision before it replaces
	// nothing its page did not show.
	draftIs(t, saveDraft(b, laptop, id, 4, text(" bug now, more"), files), 6, text(" bug now, more"), stored, tested)

	// A restore exchanges the replaced version with the draft, so nothing is
	// lost; the version it restored is not the replaced one any more.
	restored := actOnDraft(b, phone, id, "restore", 2).Expect(http.StatusOK).At("draft")
	displaced := backendtest.Map{"revision": 6, "text": text(" bug now, more"), "files": stored}
	draftIs(t, restored.(map[string]any), 7, text(" test"), stored, displaced)
	wantRefusal(t, actOnDraft(b, laptop, id, "restore", 2), http.StatusConflict, "draft_changed", "a restore of a version that went")
	dismissed := actOnDraft(b, laptop, id, "dismiss", 6).Expect(http.StatusOK).At("draft").(map[string]any)
	if dismissed["revision"] != 8.0 || dismissed["text"] != text(" test") || dismissed["replaced"] != nil {
		t.Fatalf("the dismissed draft is %v", dismissed)
	}
	wantRefusal(t, actOnDraft(b, laptop, id, "dismiss", 6), http.StatusConflict, "draft_changed", "a second dismissal")
	// A dismissal changes no text: a save built on the revision before it
	// replaces nothing its page did not show.
	restated := saveDraft(b, phone, id, 7, text(" test more"), files)
	if restated["revision"] != 9.0 || restated["replaced"] != nil {
		t.Fatalf("the restated draft is %v", restated)
	}

	// An empty draft that a stale save replaces is nothing to restore.
	emptied := saveDraft(b, laptop, id, 9, "", []any{})
	late := saveDraft(b, phone, id, 8, text(" late"), files)
	if emptied["revision"] != 10.0 || late["revision"] != 11.0 || late["replaced"] != nil {
		t.Fatalf("the emptied draft is %v and the late one %v", emptied, late)
	}

	path := "/api/conversations/" + id + "/draft"
	for _, refusal := range []struct {
		body   backendtest.Map
		status int
		code   string
	}{
		{backendtest.Map{"base": 11, "text": "one mark", "files": files}, http.StatusBadRequest, "invalid_body"},
		{backendtest.Map{"base": 9, "text": draftMark, "files": []any{backendtest.Map{"type": "text", "text": "not a file"}}}, http.StatusBadRequest, "invalid_body"},
		{backendtest.Map{"base": 9, "text": draftMark, "files": []any{
			backendtest.Map{"type": "upload", "ref": "no-such-upload", "fileName": "a.txt"},
		}}, http.StatusNotFound, "upload_not_found"},
		{backendtest.Map{"base": 9, "text": strings.Repeat("x", 256*1024), "files": []any{}}, http.StatusRequestEntityTooLarge, "too_large"},
	} {
		wantRefusal(t, b.Put(path, laptop, refusal.body), refusal.status, refusal.code, "a refused save")
	}
	backendtest.AssertJSON(t, readDraft(b, laptop, id), late)

	// An archived conversation reads its draft and refuses the rest.
	b.Patch("/api/conversations/"+id, laptop, backendtest.Map{"archived": true}).Expect(http.StatusOK)
	backendtest.AssertJSON(t, readDraft(b, phone, id), late)
	wantRefusal(t, putDraft(b, phone, id, 9, "", []any{}), http.StatusConflict, "conversation_archived", "a save to an archived conversation")
	wantRefusal(t, actOnDraft(b, phone, id, "dismiss", 9), http.StatusConflict, "conversation_archived", "a dismissal on an archived conversation")
	b.Stop()
}
