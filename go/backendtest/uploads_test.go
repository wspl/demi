package backendtest_test

import (
	"bytes"
	"encoding/base64"
	"image"
	"image/color"
	"image/png"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// attachmentMaxBytes is the most bytes of one upload.
const attachmentMaxBytes = 25 * 1024 * 1024

// pngOf is a PNG of width by height px whose pixels follow the seed: a real
// one, since an image is decoded as it enters a transcript.
func pngOf(t *testing.T, width, height int, seed byte) []byte {
	t.Helper()
	pixels := image.NewNRGBA(image.Rect(0, 0, width, height))
	for y := range height {
		for x := range width {
			pixels.Set(x, y, color.NRGBA{R: byte(x), G: byte(y), B: seed, A: 255})
		}
	}
	var encoded bytes.Buffer
	if err := png.Encode(&encoded, pixels); err != nil {
		t.Fatal(err)
	}
	return encoded.Bytes()
}

// postUpload posts bytes to the upload route, sent as the media type when there
// is one.
func postUpload(b *backendtest.Backend, session *backendtest.Session, query, mediaType string, content []byte) *backendtest.Answer {
	return b.Do(backendtest.Request{Method: http.MethodPost, Path: "/api/attachments" + query, Session: session, Raw: content, ContentType: mediaType})
}

// naming is a text and the uploads it names under their file names, as the
// composer sends them.
func naming(text string, uploads ...[2]any) []any {
	content := []any{backendtest.Map{"type": "text", "text": text}}
	for _, named := range uploads {
		attachment := named[0].(map[string]any)
		content = append(content, backendtest.Map{"type": "upload", "ref": attachment["id"], "fileName": named[1]})
	}
	return content
}

func withUpload(id, text string, uploads ...[2]any) backendtest.Frame {
	return backendtest.Frame{"type": "send", "messageId": id, "content": naming(text, uploads...)}
}

// waitForGoCall is the model's shell call that waits until the file go appears
// where the conversation works.
func waitForGoCall() *scripted.Response {
	return scripted.ToolUse("toolu_wait", "shell_exec", backendtest.Map{
		"description": "Wait", "script": "until [ -f go ]; do sleep 0.05; done", "timeoutMs": 60_000,
	})
}

// Cost: one backend, a scripted vendor and a real runner, over a second: the
// uploads reach a real device over four turns.
func TestAnUploadReachesTheModelThroughTheConversationsHostAndThePageByReference(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	ana := b.CreateUser(master, "ana@example.test", "ana-pass-1", "user")
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	paired := onDeviceConversation(t, b, master, convFirst)
	root := paired.Home()
	shot := pngOf(t, 4, 3, 0)

	// The answer says what the backend read from the bytes: a type it recognizes,
	// and a text file's opening, which the name decides.
	shotUpload := upload(t, b, master, "shot.png", "application/octet-stream", shot)
	sum := sha(shot)
	if shotUpload["mediaType"] != "image/png" || shotUpload["sizeBytes"] != float64(len(shot)) || shotUpload["sha256"] != sum || shotUpload["snippet"] != nil {
		t.Fatalf("the image's record is %v", shotUpload)
	}
	notes := upload(t, b, master, "notes.log", "application/octet-stream", []byte("\r\n  first\r\nsecond"))
	if notes["mediaType"] != "application/octet-stream" || notes["snippet"] != "first\nsecond" {
		t.Fatalf("the notes' record is %v", notes)
	}
	hers := upload(t, b, ana, "hers.txt", "text/plain", []byte("not yours"))

	// What is no single file's bytes is refused.
	for _, refusal := range []struct {
		query, mediaType string
		content          []byte
		status           int
		code             string
	}{
		{"", "image/png", shot, http.StatusBadRequest, "invalid_query"},
		{"?name=a.png", "", shot, http.StatusBadRequest, "invalid_body"},
		{"?name=a.png", "multipart/form-data; boundary=x", shot, http.StatusBadRequest, "invalid_body"},
		{"?name=a.png", "image/png", []byte{}, http.StatusBadRequest, "invalid_body"},
		{"?name=a.bin", "application/octet-stream", make([]byte, attachmentMaxBytes+1), http.StatusRequestEntityTooLarge, "too_large"},
	} {
		wantRefusal(t, postUpload(b, master, refusal.query, refusal.mediaType, refusal.content), refusal.status, refusal.code, refusal.query+" "+refusal.mediaType)
	}

	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.Answer([]string{"Seen."}, 1, 1))
	socket.Send(withUpload("m1", "Look", [2]any{shotUpload, "shot.png"}, [2]any{notes, "notes.log"}, [2]any{hers, "hers.txt"}))
	turn := socket.UntilIdle()

	// The files are on the conversation's Host, outside its working directory;
	// another user's upload is not.
	directory := filepath.Join(paired.Home(), ".demi/attachments", convFirst)
	if got, err := os.ReadFile(filepath.Join(directory, "shot.png")); err != nil || !bytes.Equal(got, shot) {
		t.Fatalf("shot.png on the Host: %v", err)
	}
	if got := readFile(t, filepath.Join(directory, "notes.log")); got != "\r\n  first\r\nsecond" {
		t.Fatalf("notes.log on the Host holds %q", got)
	}
	if _, err := os.Stat(filepath.Join(directory, "hers.txt")); err == nil {
		t.Fatal("another user's upload is on the Host")
	}
	// The model reads the image and each file's record, and learns the other
	// user's upload is not available.
	encoded := base64.StdEncoding.EncodeToString(shot)
	sent := jsonText(backendtest.At(scenarioItem(t, vendor.Requests(), 0).JSON(t), "messages"))
	contains(t, sent, encoded, directory+"/shot.png", directory+"/notes.log", "[attachment "+hers["id"].(string)+" is not available]")
	// The page receives the image by reference, never its bytes, and reads them
	// from its blobs.
	frames := jsonText(turn)
	if strings.Contains(frames, encoded) || !strings.Contains(frames, sum) {
		t.Fatal("the page does not receive the image by reference")
	}
	blocks := b.Transcript(master, convFirst)
	user := scenarioItem(t, blocks, 0)
	var kinds []string
	content, _ := backendtest.At(user, "content").([]any)
	for _, part := range content {
		kinds = append(kinds, backendtest.At(part, "type").(string))
	}
	if !slices.Equal(kinds, []string{"text", "image", "attachment", "attachment", "text"}) {
		t.Fatalf("the message's parts are %v", kinds)
	}
	if backendtest.At(content[1], "source.type") != "ref" || backendtest.At(content[1], "source.ref") != sum {
		t.Fatalf("the image is %v", content[1])
	}
	if got := b.Get("/api/blobs/"+sum, master).Expect(http.StatusOK).Body; !bytes.Equal(got, shot) {
		t.Fatal("the blob differs from the upload")
	}

	// The same name again is the next free one; nothing is overwritten.
	vendor.Respond(scripted.Answer([]string{"Again."}, 1, 1))
	socket.Send(withUpload("m2", "Once more", [2]any{shotUpload, "shot.png"}))
	socket.UntilIdle()
	if got, err := os.ReadFile(filepath.Join(directory, "shot-2.png")); err != nil || !bytes.Equal(got, shot) {
		t.Fatalf("shot-2.png on the Host: %v", err)
	}

	// A steer names an upload as a message does: the file goes to the Host, and
	// the running turn's next request reads its image and its record.
	vendor.Respond(waitForGoCall())
	vendor.Respond(scripted.Answer([]string{"Steered."}, 1, 1))
	socket.Send(backendtest.SendMessage("m3", "Wait for the file"))
	vendor.Received(t, 3)
	socket.Send(backendtest.Frame{"type": "steer", "steerId": "s1", "content": naming("And this one", [2]any{shotUpload, "shot.png"})})
	steered := socket.UntilType("steer_result")
	if backendtest.At(steered[len(steered)-1], "outcome.status") != "accepted" {
		t.Fatalf("the steer is %v", steered[len(steered)-1])
	}
	if err := os.WriteFile(filepath.Join(root, "go"), nil, 0o644); err != nil {
		t.Fatal(err)
	}
	// The turn was running before the steer's answer came.
	socket.Until(isIdle)
	if got, err := os.ReadFile(filepath.Join(directory, "shot-3.png")); err != nil || !bytes.Equal(got, shot) {
		t.Fatalf("shot-3.png on the Host: %v", err)
	}
	continued := jsonText(backendtest.At(scenarioItem(t, vendor.Requests(), 3).JSON(t), "messages"))
	contains(t, continued, "And this one", encoded, directory+"/shot-3.png")

	// An edit of the first message keeps its image by the reference the page
	// shows, and the model reads the image's bytes again.
	source, _ := backendtest.At(content[1], "source").(map[string]any)
	socket.Send(backendtest.Frame{"type": "sync_transcript"})
	synced := socket.UntilType("transcript_reset")
	keep := backendtest.Frame{"type": "edit_and_send", "request": backendtest.Map{
		"operationId": "keep-1", "targetBlockId": backendtest.At(user, "id"), "version": synced[len(synced)-1]["version"],
		"content": []any{
			backendtest.Map{"type": "text", "text": "Look again"},
			backendtest.Map{"type": "media", "media": backendtest.Map{"type": "image", "ref": source["ref"], "mediaType": source["mediaType"]}},
		},
	}}
	vendor.Respond(scripted.Answer([]string{"The same shot."}, 1, 1))
	socket.Send(keep)
	edited := socket.UntilIdle()
	if !anyFrame(edited, func(f backendtest.Frame) bool {
		return f["type"] == "edit_result" && backendtest.At(f, "outcome.status") == "accepted"
	}) {
		t.Fatalf("the edit is not accepted: %v", frameTypes(edited))
	}
	requests := vendor.Requests()
	if len(requests) != 5 {
		t.Fatalf("the replacement asks the model: %d requests", len(requests))
	}
	contains(t, jsonText(backendtest.At(requests[4].JSON(t), "messages")), "Look again", encoded)
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, about a second.
func TestAToolMediumThatCannotBeStoredIsGoneFromItsResultAndTheTurnGoesOn(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	paired := onDeviceConversation(t, b, master, convFirst)
	shot := pngOf(t, 4, 3, 0)
	if err := os.WriteFile(filepath.Join(paired.Home(), "shot.png"), shot, 0o644); err != nil {
		t.Fatal(err)
	}
	// The owner's blob namespace cannot be made, so nothing can be stored there:
	// the object store's directory blobs/<user> is a file. The data directory's
	// layout is the contract here (storage.md § Ownership and layout).
	blobs := filepath.Join(h.DataDir(), "blobs")
	if err := os.MkdirAll(blobs, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(blobs, master.User.ID), nil, 0o644); err != nil {
		t.Fatal(err)
	}
	// The directory's model reads PNG.
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()

	// The shell's stdout is a picture, which the tool's result would carry to a
	// model that reads PNG.
	vendor.Respond(scripted.ToolUse("toolu_1", "shell_exec", backendtest.Map{"description": "Show it", "script": "cat shot.png", "timeoutMs": 60_000}))
	vendor.Respond(scripted.Answer([]string{"No picture."}, 1, 1))
	turn := socket.Chat("m1", "Show me the picture")

	// The picture was not stored, so in its place the page receives a part that
	// says it is gone and why, the model reads that as its text, the turn goes
	// on, and nothing carries the picture's bytes.
	encoded := base64.StdEncoding.EncodeToString(shot)
	var gone []any
	for _, frame := range turn {
		patches, _ := frame["patches"].([]any)
		for _, patch := range patches {
			if backendtest.At(patch, "value.type") != "tool_call" {
				continue
			}
			output, _ := backendtest.At(patch, "value.output").([]any)
			for _, part := range output {
				if backendtest.At(part, "type") == "gone" {
					gone = append(gone, part)
				}
			}
		}
	}
	if len(gone) == 0 {
		t.Fatalf("no gone part reached the page: %s", jsonText(turn))
	}
	part := gone[0]
	if backendtest.At(part, "kind") != "image" || backendtest.At(part, "mediaType") != "image/png" || backendtest.At(part, "cause.type") != "not_stored" {
		t.Fatalf("the gone part is %v", part)
	}
	frames := jsonText(turn)
	if !strings.Contains(frames, "No picture.") || strings.Contains(frames, encoded) {
		t.Fatal("the turn did not go on, or carries the picture's bytes")
	}
	continued := jsonText(backendtest.At(scenarioItem(t, vendor.Requests(), 1).JSON(t), "messages"))
	text := jsonText("[image not stored: " + backendtest.At(part, "cause.error").(string) + "]")
	if !strings.Contains(continued, text) || strings.Contains(continued, encoded) {
		t.Fatalf("the model reads %s", continued)
	}
	b.Stop()
}

// firstImage is the base64 of the first image a Messages API request carries,
// in a message or a tool result.
func firstImage(t *testing.T, request any) string {
	t.Helper()
	text := jsonText(backendtest.At(request, "messages"))
	const marker = `"data":"`
	start := strings.Index(text, marker)
	if start < 0 {
		t.Fatal("the request carries no image")
	}
	start += len(marker)
	encoded, _, found := strings.Cut(text[start:], `"`)
	if !found {
		t.Fatal("the image data has no closing quote")
	}
	return encoded
}

// Cost: one backend, a scripted vendor and a real runner, about a second: an
// upload and a shell's output reach a real device.
func TestAnImageOver2000PxEntersFittedFromAnUploadAndAToolAndStaysWholeOnTheHost(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	paired := onDeviceConversation(t, b, master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	wide := pngOf(t, 2_400, 10, 1)
	if err := os.WriteFile(filepath.Join(paired.Home(), "wide.png"), wide, 0o644); err != nil {
		t.Fatal(err)
	}
	wideUpload := upload(t, b, master, "wide.png", "image/png", wide)
	socket := b.Connect(master, convFirst)
	socket.Open()

	vendor.Respond(scripted.ToolUse("toolu_1", "shell_exec", backendtest.Map{"description": "Show it", "script": "cat wide.png", "timeoutMs": 60_000}))
	vendor.Respond(scripted.Answer([]string{"Both are wide."}, 1, 1))
	socket.Send(withUpload("m1", "Look", [2]any{wideUpload, "wide.png"}))
	socket.UntilIdle()

	// The upload's file on the Host is the original, and so is the shell's
	// output; the model reads the image fitted to 2,000 px, the same bytes in
	// every request and from either source.
	directory := filepath.Join(paired.Home(), ".demi/attachments", convFirst)
	if got, err := os.ReadFile(filepath.Join(directory, "wide.png")); err != nil || !bytes.Equal(got, wide) {
		t.Fatalf("wide.png on the Host: %v", err)
	}
	requests := vendor.Requests()
	first := scenarioItem(t, requests, 0).JSON(t)
	second := scenarioItem(t, requests, 1).JSON(t)
	fitted := firstImage(t, first)
	fittedBytes, err := base64.StdEncoding.DecodeString(fitted)
	if err != nil {
		t.Fatal(err)
	}
	size, err := png.DecodeConfig(bytes.NewReader(fittedBytes))
	if err != nil {
		t.Fatal(err)
	}
	if width, height := size.Width, size.Height; width != 2_000 || height != 8 {
		t.Fatalf("the model reads an image of %d by %d", width, height)
	}
	if strings.Contains(jsonText(backendtest.At(first, "messages")), base64.StdEncoding.EncodeToString(wide)) {
		t.Fatal("the model reads the original")
	}
	if firstImage(t, second) != fitted {
		t.Fatal("the second request carries other bytes")
	}
	result := jsonText(backendtest.At(second, "messages"))
	if got := strings.Count(result, fitted); got != 2 {
		t.Fatalf("the fitted image appears %d times, not twice (the upload's and the tool's)", got)
	}
	contains(t, result, "fitted to what every model accepts; to keep the original, save it: demi shell output ")
	// The message's image is the fitted one's blob, which the page reads.
	user := scenarioItem(t, b.Transcript(master, convFirst), 0)
	ref := backendtest.At(user, "content.1.source.ref")
	if ref == nil || ref == wideUpload["sha256"] {
		t.Fatalf("the message's image is %v", backendtest.At(user, "content.1"))
	}
	if got := b.Get("/api/blobs/"+ref.(string), master).Expect(http.StatusOK).Body; !bytes.Equal(got, fittedBytes) {
		t.Fatal("the blob is not the fitted image")
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestARepeatedUploadSendsTheObjectStoreNoBytes(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	shot := pngOf(t, 4, 3, 0)
	before := b.Control.ObjectCounts()
	first := upload(t, b, master, "shot.png", "image/png", shot)
	stored := b.Control.ObjectCounts().Since(before)
	if stored.Puts != 1 || stored.BytesPut != uint64(len(shot)) {
		t.Fatalf("the first upload put %d objects of %d bytes", stored.Puts, stored.BytesPut)
	}

	// The same bytes under another name are the same blob: the object store is
	// asked whether it holds it, and receives none of its bytes again.
	before = b.Control.ObjectCounts()
	again := upload(t, b, master, "copy.png", "image/png", shot)
	repeated := b.Control.ObjectCounts().Since(before)
	if again["sha256"] != first["sha256"] {
		t.Fatalf("the same bytes are %v and %v", again["sha256"], first["sha256"])
	}
	if repeated.Puts != 0 || repeated.BytesPut != 0 || repeated.Heads != 1 {
		t.Fatalf("the repeated upload put %d objects of %d bytes after %d heads", repeated.Puts, repeated.BytesPut, repeated.Heads)
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, about a second.
func TestOpeningAStoredConversationWithImagesOnTwoPagesAndSyncingItPutsNoBlob(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	// The uploads are written to the conversation's Host.
	onDeviceConversation(t, b, master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	shots := []map[string]any{
		upload(t, b, master, "a.png", "image/png", pngOf(t, 4, 3, 1)),
		upload(t, b, master, "b.png", "image/png", pngOf(t, 4, 3, 2)),
	}
	first := b.Connect(master, convFirst)
	first.Open()
	vendor.Respond(scripted.Answer([]string{"Two shots."}, 1, 1))
	first.Send(withUpload("m1", "Look", [2]any{shots[0], "a.png"}, [2]any{shots[1], "b.png"}))
	first.UntilIdle()
	// Closing the tree leaves the conversation stored; the next open restores it.
	first.Send(backendtest.Frame{"type": "close"})
	first.UntilType("closed")

	before := b.Control.ObjectCounts()
	first.Open()
	second := b.Connect(master, convFirst)
	second.Open()
	synced := second.Live()
	// Nothing is stored again, and the object store is not even asked whether it
	// holds the images.
	after := b.Control.ObjectCounts().Since(before)
	if after.Puts != 0 || after.BytesPut != 0 || after.Heads != 0 {
		t.Fatalf("the opens put %d objects and asked %d heads", after.Puts, after.Heads)
	}
	// Every frame named the images by the references the rows hold.
	images := 0
	content, _ := backendtest.At(scenarioItem(t, synced, 0), "content").([]any)
	for _, part := range content {
		if backendtest.At(part, "type") == "image" && backendtest.At(part, "source.type") == "ref" {
			images++
		}
	}
	if images != 2 {
		t.Fatalf("the first message has %d images by reference: %v", images, scenarioItem(t, synced, 0))
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, about two seconds.
func TestARestoredConversationReadsEachReplayedBlobOnceAndNoneBeforeItsLastCompaction(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	onDeviceConversation(t, b, master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	old := upload(t, b, master, "old.png", "image/png", pngOf(t, 4, 3, 0))
	var shots [][2]any
	for shot := byte(1); shot <= 9; shot++ {
		shots = append(shots, [2]any{upload(t, b, master, "shot.png", "image/png", pngOf(t, 4, 3, shot)), "shot.png"})
	}
	socket := b.Connect(master, convFirst)
	socket.Open()
	vendor.Respond(scripted.Answer([]string{"An old shot."}, 1, 1))
	socket.Send(withUpload("m1", "Look", [2]any{old, "old.png"}))
	socket.UntilIdle()
	// The vendor refuses the nine shots' request, so the latest answered request
	// is the first: the pass summarizes what it carried, the old shot, and keeps
	// what came after it, the nine shots (compaction.md § One pass).
	refusal := backendtest.Map{"type": "error", "error": backendtest.Map{"type": "invalid_request_error", "message": "try later"}}
	vendor.Respond(scripted.Status(400).Chunk(backendtest.Marshal(refusal)))
	socket.Send(withUpload("m2", "Look at these", shots...))
	socket.UntilIdle()
	vendor.Respond(scripted.Answer([]string{"The user showed an old shot."}, 1, 1))
	socket.Send(backendtest.Frame{"type": "compact"})
	socket.UntilIdle()
	closeTree(socket)

	// The restored conversation's turn asks the model twice: an unknown tool's
	// error goes back to it.
	before := b.Control.ObjectCounts()
	socket.Open()
	if gets := b.Control.ObjectCounts().Since(before).Gets; gets != 0 {
		t.Fatalf("an open read %d blobs", gets)
	}
	vendor.Respond(scripted.ToolUse("toolu_1", "no_such_tool", backendtest.Map{}))
	vendor.Respond(scripted.Answer([]string{"Still nine."}, 1, 1))
	requests := len(vendor.Requests())
	socket.Send(backendtest.SendMessage("m3", "And now?"))
	vendor.Received(t, requests+1)
	first := b.Control.ObjectCounts().Since(before)
	socket.UntilIdle()
	both := b.Control.ObjectCounts().Since(before)

	// The first request read each replayed shot once, a few at a time, and the old
	// shot not at all; the second read nothing.
	if first.Gets != 9 || both.Gets != 9 {
		t.Fatalf("the requests read %d and %d blobs, not 9 and 9", first.Gets, both.Gets)
	}
	if both.MostGetsAtOnce <= 1 || both.MostGetsAtOnce > 8 {
		t.Fatalf("the most reads at once is %d", both.MostGetsAtOnce)
	}
	observed := vendor.Requests()
	if len(observed) < requests {
		t.Fatalf("the vendor lost recorded requests: %d, previously %d", len(observed), requests)
	}
	for _, request := range observed[requests:] {
		sent := jsonText(backendtest.At(request.JSON(t), "messages"))
		for shot := byte(1); shot <= 9; shot++ {
			if !strings.Contains(sent, base64.StdEncoding.EncodeToString(pngOf(t, 4, 3, shot))) {
				t.Fatalf("a request lacks shot %d", shot)
			}
		}
		if strings.Contains(sent, base64.StdEncoding.EncodeToString(pngOf(t, 4, 3, 0))) {
			t.Fatal("a request carries the old shot")
		}
	}
	b.Stop()
}
