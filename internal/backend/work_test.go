package backend_test

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

// filesWork is fixture state only: all page driving uses ConversationSocket.
type filesWork struct {
	ctx     context.Context
	harness *backendtest.Harness
	backend *backendtest.TestBackend
	session backendtest.Session
	vendor  *providertest.MockVendor
	paired  *backendtest.Paired
	root    string
	socket  *backendtest.ConversationSocket
	entry   string
}

// filesWorking assembles the file package and a conversation on a paired runner.
func filesWorking(t *testing.T, native string, configure ...func(*backendtest.Harness)) filesWork {
	t.Helper()
	ctx, h := conversationHarness(t)
	if native != "" {
		built, err := backendtest.BuildPackage(ctx, t, native)
		wireMust(t, err)
		wireMust(t, h.UsePackage(ctx, built))
	}
	for _, apply := range configure {
		apply(h)
	}
	vendor := providertest.StartVendor(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := conversationAnthropic(ctx, t, b, &s, vendor)
	conversationCreate(ctx, t, b, &s, filesConversation)
	paired, root := conversationOnDevice(ctx, t, h, b, &s, filesConversation)
	conversationChoose(ctx, t, b, &s, filesConversation, entry, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, b, &s, filesConversation)
	return filesWork{ctx, h, b, s, vendor, paired, root, socket, entry}
}

// filesContains checks the user-visible pieces of one command result.
func filesContains(t *testing.T, got string, wants ...string) {
	t.Helper()
	for _, want := range wants {
		if !strings.Contains(got, want) {
			t.Fatalf("missing %q in %s", want, got)
		}
	}
}

// filesModelField observes the named part of a model request, so a system
// declaration cannot accidentally satisfy an assertion about message history.
func filesModelField(t *testing.T, body []byte, name string) string {
	t.Helper()
	fields, err := contract.Object(body)
	wireMust(t, err)
	value, ok := fields[name]
	if !ok {
		t.Fatalf("model request has no %s: %s", name, body)
	}
	return string(value)
}

// filesRead observes files independently of the command that changed them.
func filesRead(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	wireMust(t, err)
	return string(data)
}

// TestWorkCreatesReadsEditsAndListsConversationFiles uses four shell jobs on a real runner; the first installs
// the file package.
func TestWorkCreatesReadsEditsAndListsConversationFiles(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-file")
	for i, step := range []struct {
		script, answer string
		wants          []string
		file           string
	}{
		{
			"mkdir src && cd src && demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF",
			"created",
			[]string{
				"exitCode: 0",
				"Created notes.md",
			},
			"alpha\nbeta\ngamma\n",
		},

		{"pwd && demi file read notes.md | grep -n a | sort -r", "read", []string{"/src\n3:gamma\n2:beta\n1:alpha"}, ""},
		{
			"demi file edit notes.md --old beta --new delta && cat notes.md",
			"edited",
			[]string{"Edited notes.md\nalpha\ndelta\ngamma"},
			"alpha\ndelta\ngamma\n",
		},

		{"ls && demi host current", "listed", []string{"notes.md", `host: machine "laptop"`}, ""},
	} {
		id := fmt.Sprintf("t%d", i+1)
		w.vendor.Respond(conversationShell(t, id, step.script, 10000))
		w.vendor.Respond(conversationAnswer(t, []string{step.answer}, 1, 1))
		_, err := w.socket.Chat(w.ctx, fmt.Sprintf("m%d", i+1), "go")
		wireMust(t, err)
		requests := w.vendor.Requests()
		filesContains(t, conversationToolResult(t, requests[len(requests)-1], id), step.wants...)
		if step.file != "" {
			conversationEqual(t, filesRead(t, filepath.Join(w.root, "src/notes.md")), step.file)
		}
	}
	filesContains(t, filesModelField(t, w.vendor.Requests()[0].Body, "system"), "demi file create", "demi host")
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// filesMove selects a named device directory through the conversation's target route.
func filesMove(
	ctx context.Context,
	t *testing.T,
	b *backendtest.TestBackend,
	s *backendtest.Session,
	id string,
	p *backendtest.Paired,
	path string,
) {
	t.Helper()
	body := `{"target":` + conversationJSON(t, &webapi.ConversationTargetDevice{DeviceID: p.ID(), Path: path}) + `}`
	conversationRequest(ctx, t, b, s, "PATCH", "/api/conversations/"+id, body, 200)
}

// TestWorkRunnerLossEndsCommandAndReconnectServesNextTurn kills a runner to end an in-flight command, then uses
// a fresh runner to read kept files.
func TestWorkRunnerLossEndsCommandAndReconnectServesNextTurn(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-file")
	w.vendor.Respond(conversationShell(t, "t1", "echo -n before > before.txt", 10000))
	w.vendor.Respond(conversationAnswer(t, []string{"written"}, 1, 1))
	_, err := w.socket.Chat(w.ctx, "m1", "go")
	wireMust(t, err)
	filesContains(t, conversationToolResult(t, w.vendor.Requests()[1], "t1"), "exitCode: 0")
	w.vendor.Respond(conversationShell(t, "t2", "touch started; sleep 20; echo late", 30000))
	w.vendor.Respond(conversationAnswer(t, []string{"the runner is gone"}, 1, 1))
	wireMust(t, w.socket.Send(w.ctx, backendtest.ConversationText("m2", "go")))
	wireMust(t, backendtest.WaitFile(w.ctx, filepath.Join(w.root, "started"), func([]byte) bool { return true }))
	wireMust(t, w.paired.Runner.Kill(w.ctx))
	_, err = w.socket.UntilIdle(w.ctx)
	wireMust(t, err)
	requests := w.vendor.Requests()
	filesContains(t, conversationToolResult(t, requests[len(requests)-1], "t2"), "exitCode: 127", "runner disconnected")
	wireMust(t, w.paired.Runner.StartAgain(w.ctx))
	wireMust(t, w.backend.UntilOnline(w.ctx, &w.session, w.paired.ID(), true))
	w.vendor.Respond(conversationShell(t, "t3", "cat before.txt", 10000))
	w.vendor.Respond(conversationAnswer(t, []string{"back"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m3", "go")
	wireMust(t, err)
	requests = w.vendor.Requests()
	filesContains(t, conversationToolResult(t, requests[len(requests)-1], "t3"), "before")
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// filesKept reads command edit histories through the stored transcript boundary.
func filesKept(
	ctx context.Context,
	t *testing.T,
	b *backendtest.TestBackend,
	s *backendtest.Session,
	id string,
) [][]core.EditedFile {
	t.Helper()
	var kept [][]core.EditedFile
	for _, block := range conversationTranscript(ctx, t, b, s, id).Blocks {
		if call, ok := block.(*core.ToolCallBlock); ok {
			if view, ok := call.View.(*core.ShellView); ok {
				var files []core.EditedFile
				if view.Files != nil {
					files = *view.Files
				}
				kept = append(kept, files)
			}
		}
	}
	return kept
}

// filesEditSides reads the two retained copies exactly as the change page does.
func filesEditSides(
	ctx context.Context,
	t *testing.T,
	b *backendtest.TestBackend,
	s *backendtest.Session,
	file core.EditedFile,
) []string {
	t.Helper()
	if len(file.Edits) == 0 || file.Edits[0].Copies == nil {
		t.Fatalf("no copies: %+v", file)
	}
	copies := file.Edits[0].Copies
	var sides []string
	for _, ref := range []core.BlobRef{copies.Original, copies.Modified} {
		sides = append(sides, string(conversationRequest(ctx, t, b, s, "GET", "/api/blobs/"+string(ref), "", 200).Body))
	}
	return sides
}

// TestWorkCommandEditsOutliveRunner uses two file-editing jobs and an archive to preserve copies after the
// runner dies.
func TestWorkCommandEditsOutliveRunner(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-file")
	script := "set -e\nprintf 'before\\n' > note.txt\nprintf 'created\\n' | demi file create native.txt\n" +
		"printf '\\0binary' > asset.bin\nprintf 'temporary' > removed.txt\nrm removed.txt"
	w.vendor.Respond(conversationShell(t, "create", script, 10000))
	w.vendor.Respond(conversationAnswer(t, []string{"Created the files."}, 1, 1))
	_, err := w.socket.Chat(w.ctx, "m1", "go")
	wireMust(t, err)
	first := filesKept(w.ctx, t, w.backend, &w.session, filesConversation)[0]
	var names []string
	for _, file := range first {
		names = append(names, filepath.Base(file.Path))
	}
	conversationEqual(t, names, []string{"note.txt", "native.txt", "asset.bin"})
	if first[2].Edits[0].Copies != nil {
		t.Fatal("binary edit has copies")
	}
	conversationEqual(t, filesEditSides(w.ctx, t, w.backend, &w.session, first[0]), []string{"", "before\n"})
	patch := "demi file patch <<'PATCH'\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-before\n+after\n" +
		"--- a/native.txt\n+++ b/native.txt\n@@ -1 +1 @@\n-created\n+patched\nPATCH"
	w.vendor.Respond(conversationShell(t, "patch", patch, 10000))
	w.vendor.Respond(conversationAnswer(t, []string{"Patched both files."}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m2", "go")
	wireMust(t, err)
	calls := filesKept(w.ctx, t, w.backend, &w.session, filesConversation)
	second := calls[len(calls)-1]
	var kinds []core.EditKind
	for _, file := range second {
		kinds = append(kinds, file.Kind)
	}
	conversationEqual(t, kinds, []core.EditKind{core.EditKindModified, core.EditKindModified})
	conversationEqual(t, filesEditSides(w.ctx, t, w.backend, &w.session, second[0]), []string{"before\n", "after\n"})
	conversationRequest(
		w.ctx,
		t,
		w.backend,
		&w.session,
		"PATCH",
		"/api/conversations/"+filesConversation,
		`{"archived":true}`,
		200,
	)
	wireMust(t, w.paired.Runner.Kill(w.ctx))
	conversationEqual(t, filesEditSides(w.ctx, t, w.backend, &w.session, second[1]), []string{"created\n", "patched\n"})
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestWorkBackendRestartReconnectsRunnerAndInterruptsTurn restarts while a runner job is blocked; its successor
// preserves files and IDs.
func TestWorkBackendRestartReconnectsRunnerAndInterruptsTurn(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-file")
	w.vendor.Respond(conversationShell(t, "t1", "echo -n kept > kept.txt && cat kept.txt", 10000))
	w.vendor.Respond(conversationAnswer(t, []string{"remember me"}, 1, 1))
	_, err := w.socket.Chat(w.ctx, "m1", "go")
	wireMust(t, err)
	kept := conversationToolResult(t, w.vendor.Requests()[1], "t1")
	filesContains(t, kept, "kept")
	conversationEqual(t, toolstest.Field(kept, "commandId"), "1")
	w.vendor.Respond(conversationShell(t, "t2", "touch started; sleep 20", 30000))
	wireMust(t, w.socket.Send(w.ctx, backendtest.ConversationText("m2", "go")))
	wireMust(t, backendtest.WaitFile(w.ctx, filepath.Join(w.root, "started"), func([]byte) bool { return true }))
	address := w.backend.Address()
	wireMust(t, w.backend.Close(w.ctx))
	w.backend, err = w.harness.StartAt(w.ctx, t, address)
	wireMust(t, err)
	wireMust(t, w.backend.UntilOnline(w.ctx, &w.session, w.paired.ID(), true))
	blocks := conversationTranscript(w.ctx, t, w.backend, &w.session, filesConversation).Blocks
	kinds := conversationKinds(t, blocks)
	last := 0
	for i, kind := range kinds {
		if kind == "user" {
			last = i
		}
	}
	conversationEqual(t, kinds[last:], []string{"user", "tool_call", "response", "error"})
	cut, ok := blocks[last+1].(*core.ToolCallBlock)
	if !ok {
		t.Fatal("missing interrupted tool call")
	}
	conversationEqual(t, string(cut.Status), "error")
	record, ok := blocks[len(blocks)-1].(*core.ErrorBlock)
	if !ok || record.Code == nil || *record.Code != "interrupted" {
		t.Fatalf("interruption: %+v", record)
	}
	conversationChoose(w.ctx, t, w.backend, &w.session, filesConversation, w.entry, "claude-opus-4-8")
	w.socket = conversationOpen(w.ctx, t, w.backend, &w.session, filesConversation)
	w.vendor.Respond(conversationShell(t, "t3", "cat kept.txt && demi host current", 10000))
	w.vendor.Respond(conversationAnswer(t, []string{"and again"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m3", "go")
	wireMust(t, err)
	requests := w.vendor.Requests()
	filesContains(t, conversationToolResult(t, requests[len(requests)-1], "t2"), "Tool call aborted")
	after := conversationToolResult(t, requests[len(requests)-1], "t3")
	filesContains(t, after, "kept", `host: machine "laptop"`)
	conversationEqual(t, toolstest.Field(after, "commandId"), "3")
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestWorkSwitchKeepsDepartedFilesReachable uses two paired targets to retain separate files; switch and
// attachment news reach the model once.
func TestWorkSwitchKeepsDepartedFilesReachable(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-file")
	beta, err := w.backend.Pair(w.ctx, t, &w.session, "beta")
	wireMust(t, err)
	onBeta := filepath.Join(beta.Runner.Home(), "project")
	wireMust(t, os.MkdirAll(onBeta, 0o755))
	notes := "demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF"
	w.vendor.Respond(conversationShell(t, "t1", notes, 10000))
	w.vendor.Respond(conversationAnswer(t, []string{"created"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m1", "go")
	wireMust(t, err)
	filesContains(t, conversationToolResult(t, w.vendor.Requests()[1], "t1"), "Created notes.md")
	filesMove(w.ctx, t, w.backend, &w.session, filesConversation, beta, onBeta)
	before := len(w.vendor.Requests())
	w.vendor.Respond(conversationShell(t, "t2", "cat notes.md; echo exit=$?", 10000))
	w.vendor.Respond(conversationShell(t, "t3", notes, 10000))
	w.vendor.Respond(conversationAnswer(t, []string{"recreated"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m2", "go")
	wireMust(t, err)
	requests := w.vendor.Requests()
	context := filesModelField(t, requests[before].Body, "messages")
	told := strings.Count(context, "[Execution context ")
	switches := strings.Count(context, "[Execution target switched]")
	filesContains(
		t,
		context,
		"[Execution target switched]",
		`Previous target: the machine \"laptop\"`,
		`stays attached as \"laptop\"`,
	)
	filesContains(t, conversationToolResult(t, requests[len(requests)-1], "t2"), "No such file or directory", "exit=1")
	filesContains(t, conversationToolResult(t, requests[len(requests)-1], "t3"), "Created notes.md")
	before = len(requests)
	w.vendor.Respond(
		conversationShell(t, "t4", "demi file edit notes.md --old beta --new delta && cat notes.md", 10000),
	)
	w.vendor.Respond(conversationAnswer(t, []string{"edited"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m3", "go")
	wireMust(t, err)
	requests = w.vendor.Requests()
	filesContains(t, conversationToolResult(t, requests[len(requests)-1], "t4"), "alpha\ndelta\ngamma")
	conversationEqual(t, filesRead(t, filepath.Join(onBeta, "notes.md")), "alpha\ndelta\ngamma\n")
	conversationEqual(t, filesRead(t, filepath.Join(w.root, "notes.md")), "alpha\nbeta\ngamma\n")
	conversationEqual(
		t,
		strings.Count(filesModelField(t, requests[before].Body, "messages"), "[Execution context "),
		told,
	)
	conversationRequest(
		w.ctx,
		t,
		w.backend,
		&w.session,
		"PATCH",
		"/api/conversations/"+filesConversation+"/hosts/"+string(w.paired.ID()),
		`{"name":"first"}`,
		200,
	)
	w.vendor.Respond(conversationAnswer(t, []string{"noted"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m4", "go")
	wireMust(t, err)
	requests = w.vendor.Requests()
	noted := filesModelField(t, requests[len(requests)-1].Body, "messages")
	filesContains(t, noted, "[Attached hosts changed]", `\"first\" (online, shells start in`)
	conversationEqual(t, strings.Count(noted, "[Execution target switched]"), switches)
	filesMove(w.ctx, t, w.backend, &w.session, filesConversation, w.paired, w.root)
	before = len(requests)
	w.vendor.Respond(conversationShell(t, "t5", `cat notes.md && demi host shell --host beta "cat notes.md"`, 20000))
	w.vendor.Respond(conversationAnswer(t, []string{"back"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m5", "go")
	wireMust(t, err)
	requests = w.vendor.Requests()
	filesContains(t, filesModelField(t, requests[before].Body, "messages"), "[Execution target switched]")
	filesContains(
		t,
		conversationToolResult(t, requests[len(requests)-1], "t5"),
		"alpha\nbeta\ngamma\nalpha\ndelta\ngamma",
	)
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestWorkConversationsKeepDirectoriesShellsAndTodosApart runs concurrent turns sharing a device but retaining
// separate cwd, environment and todos.
func TestWorkConversationsKeepDirectoriesShellsAndTodosApart(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-file")
	otherVendor := providertest.StartVendor(t)
	entry := conversationAnthropic(w.ctx, t, w.backend, &w.session, otherVendor)
	conversationCreate(w.ctx, t, w.backend, &w.session, conversationSecond)
	filesMove(w.ctx, t, w.backend, &w.session, conversationSecond, w.paired, w.root)
	conversationChoose(w.ctx, t, w.backend, &w.session, conversationSecond, entry, "claude-opus-4-8")
	other := conversationOpen(w.ctx, t, w.backend, &w.session, conversationSecond)
	scripts := [][2]string{
		{
			`mkdir -p sub && cd sub && MARK=from-a && echo "a: $(pwd) $MARK" && ` +
				`demi todo add "draft the outline" && demi todo add "run the suite"`,
			`echo "b: $(pwd) mark=${MARK:-unset}"`,
		},
		{
			`echo "a: $(pwd) mark=${MARK:-unset}" && demi todo list --json`,
			`echo "b: $(pwd) mark=${MARK:-unset}" && demi todo list --json`,
		},
	}
	for i, pair := range scripts {
		aID, bID := fmt.Sprintf("a%d", i+1), fmt.Sprintf("b%d", i+1)
		w.vendor.Respond(conversationShell(t, aID, pair[0], 10000))
		w.vendor.Respond(conversationAnswer(t, []string{"a"}, 1, 1))
		otherVendor.Respond(conversationShell(t, bID, pair[1], 10000))
		otherVendor.Respond(conversationAnswer(t, []string{"b"}, 1, 1))
		wireMust(t, w.socket.Send(w.ctx, backendtest.ConversationText(aID, "go")))
		wireMust(t, other.Send(w.ctx, backendtest.ConversationText(bID, "go")))
		_, err := w.socket.UntilIdle(w.ctx)
		wireMust(t, err)
		_, err = other.UntilIdle(w.ctx)
		wireMust(t, err)
		aRequests, bRequests := w.vendor.Requests(), otherVendor.Requests()
		a := conversationToolResult(t, aRequests[len(aRequests)-1], aID)
		b := conversationToolResult(t, bRequests[len(bRequests)-1], bID)
		if i == 0 {
			filesContains(t, a, "a: "+w.root+"/sub from-a")
		} else {
			filesContains(t, a, "a: "+w.root+"/sub mark=unset", "draft the outline", "run the suite")
			filesContains(t, b, `{"todos":[]}`)
		}
		filesContains(t, b, "b: "+w.root+" mark=unset")
	}
	wireMust(t, other.Close(w.ctx))
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestWorkHostShellPipesCarryBytesAndKeepFarDirectory uses two real runners to transfer 300 KiB through far-host
// pipes and preserve its cwd.
func TestWorkHostShellPipesCarryBytesAndKeepFarDirectory(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-file")
	beta, err := w.backend.Pair(w.ctx, t, &w.session, "beta")
	wireMust(t, err)
	b := beta.Runner.Home()
	filesMove(w.ctx, t, w.backend, &w.session, filesConversation, beta, b)
	a := w.root
	payload := backendtest.Pattern(300*1024, 0)
	wireMust(t, os.WriteFile(filepath.Join(a, "notes.bin"), payload, 0o644))
	scripts := []string{
		fmt.Sprintf(
			`demi host list && demi host shell --host laptop "tar c -C %s notes.bin" | tar x && `+
				`cmp notes.bin %s/notes.bin && echo copied`,
			a,
			a,
		),
		fmt.Sprintf(
			`head -c 250000 notes.bin > push.bin && tar c push.bin | demi host shell --host laptop "tar x -C %s" && `+
				`cmp push.bin %s/push.bin && echo pushed`,
			a,
			a,
		),
		fmt.Sprintf(
			`demi host shell --host laptop "mkdir -p sub && cd sub && pwd" && demi host shell --host %s "pwd" && demi host list`,
			w.paired.ID(),
		),
		`demi host shell --host nope "echo hi"; echo exit=$?`,
	}
	for i, script := range scripts {
		id := fmt.Sprintf("t%d", i+1)
		w.vendor.Respond(conversationShell(t, id, script, 30000))
		w.vendor.Respond(conversationAnswer(t, []string{"done"}, 1, 1))
		_, err := w.socket.Chat(w.ctx, id, "go")
		wireMust(t, err)
		requests := w.vendor.Requests()
		result := conversationToolResult(t, requests[len(requests)-1], id)
		switch i {
		case 0:
			filesContains(
				t,
				result,
				fmt.Sprintf("beta  %s  online  %s  (main)", beta.ID(), b),
				fmt.Sprintf("laptop  %s  online  %s  (attached)", w.paired.ID(), a),
				"copied",
			)
			conversationEqual(t, []byte(filesRead(t, filepath.Join(b, "notes.bin"))), payload)
			wireMust(t, backendtest.WaitRunnerJobsRemoved(w.ctx, w.paired.Runner.StateDir()))
		case 1:
			filesContains(t, result, "pushed")
			conversationEqual(t, []byte(filesRead(t, filepath.Join(a, "push.bin"))), payload[:250000])
		case 2:
			filesContains(
				t,
				result,
				a+"/sub\n"+a+"/sub\n",
				fmt.Sprintf("laptop  %s  online  %s/sub  (attached)", w.paired.ID(), a),
			)
		case 3:
			filesContains(t, result, "host nope is not reachable", "exit=1")
		}
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestWorkHostShellStreamsErrorsAcceptsInputAndStopsFarJob uses a far-host reader to stream stderr, accept
// input, and die with its caller's abort.
func TestWorkHostShellStreamsErrorsAcceptsInputAndStopsFarJob(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "demi-file")
	beta, err := w.backend.Pair(w.ctx, t, &w.session, "beta")
	wireMust(t, err)
	filesMove(w.ctx, t, w.backend, &w.session, filesConversation, beta, beta.Runner.Home())
	far := `printf "ready\n" >&2; read line; printf "%s" "$line" > got.txt; ` +
		`sh -c "echo \$\$ > far.pid; exec /bin/sleep 30"`
	w.vendor.Respond(conversationShell(t, "start", "demi host shell --host laptop '"+far+"'", 500))
	w.vendor.Respond(conversationAnswer(t, []string{"waiting"}, 1, 1))
	frames, err := w.socket.Chat(w.ctx, "m1", "go")
	wireMust(t, err)
	requests := w.vendor.Requests()
	started := conversationToolResult(t, requests[len(requests)-1], "start")
	if !strings.HasPrefix(started, "status: running\n") {
		t.Fatal(started)
	}
	command := core.CommandID(toolstest.Field(started, "commandId"))
	ready := false
	for _, f := range frames {
		if v, ok := f.(*framewire.ShellOutputFrame); ok && v.Status.Command().CommandID == command &&
			strings.Contains(v.Status.Command().Tail, "ready\n") {
			ready = true
		}
	}
	if !ready {
		filesView(
			w.ctx,
			t,
			w.socket,
			command,
			func(s framewire.ShellStatus) bool { return strings.Contains(s.Command().Tail, "ready\n") },
		)
	}
	if !strings.Contains(toolstest.ShownOutput(started), "ready") {
		w.vendor.Respond(conversationToolUse(t, "check", "shell_status", `{"commandId":"`+string(command)+`"}`))
		w.vendor.Respond(conversationAnswer(t, []string{"checked"}, 1, 1))
		_, err = w.socket.Chat(w.ctx, "m2", "check")
		wireMust(t, err)
		requests = w.vendor.Requests()
		checked := conversationToolResult(t, requests[len(requests)-1], "check")
		if !strings.HasPrefix(checked, "status: running") {
			t.Fatal(checked)
		}
		filesContains(t, toolstest.ShownOutput(started)+toolstest.ShownOutput(checked), "ready")
	}
	w.vendor.Respond(
		conversationToolUse(t, "write", "shell_write", `{"commandId":"`+string(command)+`","stdin":"hello\n"}`),
	)
	w.vendor.Respond(conversationAnswer(t, []string{"written"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m3", "write")
	wireMust(t, err)
	wireMust(
		t,
		backendtest.WaitFile(
			w.ctx,
			filepath.Join(w.root, "got.txt"),
			func(b []byte) bool { return string(b) == "hello" },
		),
	)
	pidPath := filepath.Join(w.root, "far.pid")
	wireMust(t, backendtest.WaitFile(w.ctx, pidPath, func(b []byte) bool { return strings.HasSuffix(string(b), "\n") }))
	pid, err := strconv.Atoi(strings.TrimSpace(filesRead(t, pidPath)))
	wireMust(t, err)
	running, err := backendtest.ProcessRunning(pid)
	wireMust(t, err)
	if !running {
		t.Fatal("far process is not running")
	}
	w.vendor.Respond(conversationToolUse(t, "abort", "shell_abort", `{"commandId":"`+string(command)+`"}`))
	w.vendor.Respond(conversationAnswer(t, []string{"stopped"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "m4", "stop")
	wireMust(t, err)
	requests = w.vendor.Requests()
	aborted := conversationToolResult(t, requests[len(requests)-1], "abort")
	if !strings.HasPrefix(aborted, "status: aborted\n") {
		t.Fatal(aborted)
	}
	wireMust(t, backendtest.WaitRunnerJobsRemoved(w.ctx, w.paired.Runner.StateDir()))
	if running, err := backendtest.ProcessRunning(pid); err != nil || running {
		t.Fatalf("far process remains: %v", err)
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}
