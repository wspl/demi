package backendtest_test

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// Cost: one backend and a real runner that installs the native fixture, about
// half a second.
func TestAnArchiveSucceedsThoughItsDeviceGoesAwayDuringTheRelease(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithNativeFixture()).StartSetUp()
	laptop := b.Pair(master, "laptop")
	for _, id := range []string{convFirst, convSecond} {
		b.CreateConversation(master, id)
		b.SwitchTo(master, id, laptop, laptop.Home())
	}
	// The fixture service holds the conversation, and its release of it never
	// ends by itself.
	_, code, reason := b.OpenStream(master, convFirst, "stall_release").Received()
	if code != 1000 || reason != "completed" {
		t.Fatalf("the stream closes with %d %s", code, reason)
	}

	// The archive's release reaches the device, which goes away before it answers;
	// the archive succeeds all the same.
	archived := make(chan *backendtest.Answer, 1)
	failed := make(chan error, 1)
	go func() {
		answer, err := b.TryDo(backendtest.Request{
			Method: http.MethodPatch, Path: "/api/conversations/" + convFirst, Session: master,
			Body: backendtest.Map{"archived": true},
		})
		if err != nil {
			failed <- err
			return
		}
		archived <- answer
	}()
	_, code, reason = b.OpenStream(master, convSecond, "stalled").Received()
	if code != 1000 || reason != "completed" {
		t.Fatalf("the stream closes with %d %s", code, reason)
	}
	laptop.Runner.Kill()
	select {
	case answer := <-archived:
		answer.Expect(http.StatusOK)
	case err := <-failed:
		t.Fatal(err)
	}
	b.Stop()
}

const oneDay = 24 * time.Hour

// blobPath is where the object store of the data directory keeps bytes in the
// user's namespace. The data directory's layout is the contract here
// (storage.md § Ownership and layout): retention collects the objects the
// backend wrote there.
func blobPath(h *backendtest.Harness, session *backendtest.Session, content []byte) string {
	sum := sha256.Sum256(content)
	return filepath.Join(h.DataDir(), "blobs", session.User.ID, hex.EncodeToString(sum[:]))
}

// orphan stores content in the user's namespace with its object written age
// before now, as a put whose save failed leaves it.
func orphan(t *testing.T, h *backendtest.Harness, session *backendtest.Session, content []byte, now time.Time, age time.Duration) string {
	t.Helper()
	path := blobPath(h, session, content)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, content, 0o644); err != nil {
		t.Fatal(err)
	}
	written := now.Add(-age)
	if err := os.Chtimes(path, written, written); err != nil {
		t.Fatal(err)
	}
	return path
}

// openOnDevice opens a conversation on a paired device whose model reads PNG,
// and answers its socket, the directory it works in and the device.
func openOnDevice(t *testing.T, b *backendtest.Backend, master *backendtest.Session, vendor *scripted.Vendor) (*backendtest.Socket, string, *backendtest.Paired) {
	t.Helper()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	paired := onDeviceConversation(t, b, master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()
	return socket, paired.Home(), paired
}

// closeTree closes the page's tree, which disposes it.
func closeTree(socket *backendtest.Socket) {
	socket.Send(backendtest.Frame{"type": "close"})
	socket.UntilType("closed")
}

// resultMedia is the media of the result of the tool call id in the stored
// history: each image's blob, or, where an image was retired, its media type and
// the UTC day of its retirement.
func resultMedia(t *testing.T, b *backendtest.Backend, master *backendtest.Session, id string) []string {
	t.Helper()
	var media []string
	found := false
	for _, block := range b.Transcript(master, convFirst) {
		if backendtest.At(block, "type") != "tool_call" || backendtest.At(block, "toolUseId") != id {
			continue
		}
		found = true
		parts, _ := backendtest.At(block, "output").([]any)
		for _, part := range parts {
			switch backendtest.At(part, "type") {
			case "image":
				if ref, ok := backendtest.At(part, "source.ref").(string); ok {
					media = append(media, ref)
				}
			case "gone":
				if backendtest.At(part, "kind") == "image" && backendtest.At(part, "cause.type") == "retired" {
					at, _ := backendtest.At(part, "cause.at").(string)
					day := instant(t, at).UTC().Format(time.DateOnly)
					media = append(media, fmt.Sprintf("%v retired on %s", backendtest.At(part, "mediaType"), day))
				}
			}
		}
	}
	if !found {
		t.Fatalf("no call %s in the history", id)
	}
	return media
}

func shaOfPNG(content []byte) string {
	sum := sha256.Sum256(content)
	return hex.EncodeToString(sum[:])
}

// Cost: one backend started twice, about two seconds.
func TestABackendRunsItsFirstRetentionPassOnceItServes(t *testing.T) {
	t.Parallel()
	now := time.Now()
	h := backendtest.New(t, backendtest.WithClockAt(now))
	h.Lifecycle().RetentionIntervalMs = backendtest.Ptr(uint64(oneDay.Milliseconds()))
	b, master := h.StartSetUp()
	b.Stop()
	left := orphan(t, h, master, pngOf(t, 4, 3, 4), now, oneDay+time.Hour)

	b = h.Start()
	backendtest.Eventually(t, "the first pass collects the blob", func() bool {
		_, err := os.Stat(left)
		return err != nil
	})
	if deletes := b.Control.ObjectCounts().Deletes; deletes != 1 {
		t.Fatalf("the pass deleted %d objects", deletes)
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, about two seconds.
func TestACollectionDeletesAnUnreferencedBlobPastTheGraceAndNothingWhileADatabaseCannotBeRead(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	now := time.Now()
	h := backendtest.New(t, backendtest.WithClockAt(now))
	b, master := h.StartSetUp()
	socket, root, _ := openOnDevice(t, b, master, vendor)
	// A block references a tool's screenshot, and another the copies of a file its
	// command created: the empty file before and the note after.
	writeFile(t, filepath.Join(root, "shot.png"), string(pngOf(t, 4, 3, 1)))
	vendor.Respond(backendtest.ShellCall("toolu_0", "printf 'noted\\n' > note.txt", time.Minute))
	vendor.Respond(backendtest.ShellCall("toolu_1", "cat shot.png", time.Minute))
	vendor.Respond(backendtest.Say("Seen."))
	socket.Chat("m1", "Show me the shot")
	// An upload no message names, and one a draft of another conversation stages.
	upload(t, b, master, "kept.png", "image/png", pngOf(t, 4, 3, 2))
	staged := upload(t, b, master, "staged.png", "image/png", pngOf(t, 4, 3, 3))
	b.CreateConversation(master, convSecond)
	files := []any{backendtest.Map{"type": "upload", "ref": staged["id"], "fileName": "staged.png"}}
	b.Put("/api/conversations/"+convSecond+"/draft", master, backendtest.Map{"base": 0, "text": "\uFFFC", "files": files}).Expect(http.StatusOK)
	// What a failed save left a minute more than the grace ago, and what
	// another left a minute less: the grace is a day.
	now = b.Control.AdvanceClock(oneDay + time.Hour)
	left := orphan(t, h, master, pngOf(t, 4, 3, 4), now, oneDay+time.Minute)
	recent := orphan(t, h, master, pngOf(t, 4, 3, 5), now, oneDay-time.Minute)
	kept := []string{recent}
	for _, content := range [][]byte{pngOf(t, 4, 3, 1), pngOf(t, 4, 3, 2), pngOf(t, 4, 3, 3), {}, []byte("noted\n")} {
		kept = append(kept, blobPath(h, master, content))
	}

	// The other conversation's database cannot be read, so the collection deletes
	// nothing. The layout of the data directory is the contract here too.
	database := filepath.Join(h.DataDir(), "conversations", convSecond+".sqlite")
	writeFile(t, database, "not a database")
	// The master's session ended with the day that passed.
	master = b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	before := b.Control.ObjectCounts()
	b.Control.RunRetention(master.User.ID)
	if deletes := b.Control.ObjectCounts().Since(before).Deletes; deletes != 0 {
		t.Fatalf("a pass over an unreadable database deleted %d objects", deletes)
	}
	if _, err := os.Stat(left); err != nil {
		t.Fatal("the blob was collected while a database could not be read")
	}

	// Once every source can be read, only the blob that nothing references and
	// that is past the grace goes.
	if err := os.Remove(database); err != nil {
		t.Fatal(err)
	}
	before = b.Control.ObjectCounts()
	b.Control.RunRetention(master.User.ID)
	collected := b.Control.ObjectCounts().Since(before)
	if collected.Lists != 1 || collected.Deletes != 1 {
		t.Fatalf("the pass listed %d times and deleted %d objects", collected.Lists, collected.Deletes)
	}
	if _, err := os.Stat(left); err == nil {
		t.Fatal("the unreferenced blob past the grace stays")
	}
	for _, path := range kept {
		if _, err := os.Stat(path); err != nil {
			t.Fatalf("%s is gone: %v", path, err)
		}
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, about three seconds: a
// real device runs two shell commands, and the conversation compacts.
func TestAToolImageSummarizedADayBeforeGoesAfter30DaysOnceItsPageLetsGoAndItsBlobADayLater(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	now := time.Now()
	h := backendtest.New(t, backendtest.WithClockAt(now))
	b, master := h.StartSetUp()
	socket, root, _ := openOnDevice(t, b, master, vendor)
	shotBytes, laterBytes, pastedBytes := pngOf(t, 4, 3, 1), pngOf(t, 4, 3, 2), pngOf(t, 4, 3, 3)
	writeFile(t, filepath.Join(root, "shot.png"), string(shotBytes))
	writeFile(t, filepath.Join(root, "later.png"), string(laterBytes))
	pasted := upload(t, b, master, "pasted.png", "image/png", pastedBytes)
	vendor.Respond(backendtest.ShellCall("toolu_1", "cat shot.png", time.Minute))
	vendor.Respond(backendtest.Say("Seen."))
	socket.Send(withUpload("m1", "Look", [2]any{pasted, "pasted.png"}))
	socket.UntilIdle()
	// A long message fills the history compaction keeps, so the pass summarizes
	// the first turn: its images lie before the boundary.
	vendor.Respond(backendtest.Say("Read."))
	socket.Chat("m2", strings.Repeat("a long note ", 2_000))
	vendor.Respond(scripted.Answer([]string{"The user showed two images."}, 1, 1))
	socket.Send(backendtest.Frame{"type": "compact"})
	socket.UntilIdle()
	vendor.Respond(backendtest.ShellCall("toolu_2", "cat later.png", time.Minute))
	vendor.Respond(backendtest.Say("Seen again."))
	socket.Chat("m3", "And the later one")
	shot, later := shaOfPNG(shotBytes), shaOfPNG(laterBytes)

	// Thirty-one days later, a page still has the conversation open: its tree
	// holds the history, so nothing is retired. The web session of a month ago
	// has expired.
	now = b.Control.AdvanceClock(31 * oneDay)
	master = b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	b.Control.RunRetention(master.User.ID)
	if got := resultMedia(t, b, master, "toolu_1"); !slices.Equal(got, []string{shot}) {
		t.Fatalf("the open conversation's image is %v", got)
	}

	// Once the page lets go, the image before the day-old boundary goes. The one
	// in the replayed window stays, since the conversation was in use, and so does
	// the message's.
	closeTree(socket)
	retired := "image/png retired on " + now.UTC().Format(time.DateOnly)
	backendtest.Eventually(t, "the summarized image is retired", func() bool {
		return slices.Equal(resultMedia(t, b, master, "toolu_1"), []string{retired})
	})
	if got := resultMedia(t, b, master, "toolu_2"); !slices.Equal(got, []string{later}) {
		t.Fatalf("the replayed image is %v", got)
	}
	blocks := b.Transcript(master, convFirst)
	message := backendtest.At(scenarioItem(t, blocks, 0), "content")
	if !strings.Contains(jsonText(message), `"image"`) {
		t.Fatalf("the message's image is gone: %v", message)
	}

	// The retirement used the blob; a day later nothing did, and it goes, with the
	// outputs of the two commands, which the same pass removed 30 days after their
	// commands ended.
	b.Control.AdvanceClock(oneDay + time.Hour)
	before := b.Control.ObjectCounts()
	b.Control.RunRetention(master.User.ID)
	if deletes := b.Control.ObjectCounts().Since(before).Deletes; deletes != 3 {
		t.Fatalf("the pass deleted %d objects", deletes)
	}
	if _, err := os.Stat(blobPath(h, master, shotBytes)); err == nil {
		t.Fatal("the retired image's blob stays")
	}
	for _, content := range [][]byte{laterBytes, pastedBytes} {
		if _, err := os.Stat(blobPath(h, master, content)); err != nil {
			t.Fatalf("a blob in use is gone: %v", err)
		}
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, about two seconds.
func TestAConversationIdleFor30DaysLosesItsToolImagesAndItsNextRequestCarriesTheirText(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	now := time.Now()
	b, master := backendtest.New(t, backendtest.WithClockAt(now)).StartSetUp()
	socket, root, _ := openOnDevice(t, b, master, vendor)
	shotBytes := pngOf(t, 4, 3, 1)
	writeFile(t, filepath.Join(root, "shot.png"), string(shotBytes))
	vendor.Respond(backendtest.ShellCall("toolu_1", "cat shot.png", time.Minute))
	vendor.Respond(backendtest.Say("Seen."))
	socket.Chat("m1", "Show me the shot")
	closeTree(socket)

	now = b.Control.AdvanceClock(31 * oneDay)
	master = b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	b.Control.RunRetention(master.User.ID)
	day := now.UTC().Format(time.DateOnly)
	if got := resultMedia(t, b, master, "toolu_1"); !slices.Equal(got, []string{"image/png retired on " + day}) {
		t.Fatalf("the idle conversation's image is %v", got)
	}

	// The resumed conversation's next request carries the text where the image
	// was.
	socket.Open()
	vendor.Respond(backendtest.Say("Still here."))
	socket.Chat("m2", "Anything new?")
	requests := vendor.Requests()
	sent := jsonText(backendtest.At(scenarioItem(t, requests, len(requests)-1).JSON(t), "messages"))
	retired := "[image:image/png, removed on " + day + ": a tool result's images and videos are kept for 30 days]"
	if !strings.Contains(sent, retired) {
		t.Fatalf("the request lacks %q:\n%s", retired, sent)
	}
	if strings.Contains(sent, base64.StdEncoding.EncodeToString(shotBytes)) {
		t.Fatal("the request still carries the image")
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, about two seconds.
func TestACommandsOutputIsRemoved30DaysAfterItEnded(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	now := time.Now()
	b, master := backendtest.New(t, backendtest.WithClockAt(now)).StartSetUp()
	socket, _, _ := openOnDevice(t, b, master, vendor)
	vendor.Respond(backendtest.ShellCall("toolu_1", "echo kept", time.Minute))
	vendor.Respond(backendtest.Say("Said."))
	socket.Chat("m1", "Say it")
	command := backendtest.Field(t, scripted.ToolResult(t, scenarioItem(t, vendor.Requests(), 1).JSON(t), "toolu_1"), "commandId")
	read := func(id string) *scripted.Response {
		return backendtest.ShellCall(id, "demi shell output "+command+" --raw", time.Minute)
	}
	vendor.Respond(read("toolu_2"))
	vendor.Respond(backendtest.Say("Read."))
	socket.Chat("m2", "Read it")
	requests := vendor.Requests()
	if got := scripted.ToolResult(t, scenarioItem(t, requests, len(requests)-1).JSON(t), "toolu_2"); !strings.Contains(got, "kept") {
		t.Fatalf("the output reads %q", got)
	}

	// Thirty-one days later, the retention pass removes it, with the day.
	now = b.Control.AdvanceClock(31 * oneDay)
	master = b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	b.Control.RunRetention(master.User.ID)
	vendor.Respond(read("toolu_3"))
	vendor.Respond(backendtest.Say("Gone."))
	socket.Chat("m3", "Read it again")
	requests = vendor.Requests()
	gone := scripted.ToolResult(t, scenarioItem(t, requests, len(requests)-1).JSON(t), "toolu_3")
	removed := "demi shell output: the output of " + command + " was removed on " + now.UTC().Format(time.DateOnly) + ", 30 days after the command ended"
	if !strings.Contains(gone, removed) {
		t.Fatalf("the output reads %q, not %q", gone, removed)
	}
	b.Stop()
}
