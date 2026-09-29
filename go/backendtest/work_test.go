package backendtest_test

import (
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// The conversation ids the work scenarios create.
const (
	workFirst  = "1e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	workSecond = "2e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a02"
)

// contextBlock opens each context block the model reads once the conversation's
// execution context changed, and targetSwitched the line that opens a context
// block's announcement of a target switch.
const (
	contextBlock   = "[Execution context "
	targetSwitched = "[Execution target switched]"
)

// directoryIn makes a directory in the device's home.
func directoryIn(t *testing.T, device *backendtest.Paired, name string) string {
	t.Helper()
	path := filepath.Join(device.Home(), name)
	if err := os.MkdirAll(path, 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

// untilExists waits until something is at path.
func untilExists(t *testing.T, path string) {
	t.Helper()
	backendtest.Eventually(t, path+" exists", func() bool {
		_, err := os.Stat(path)
		return err == nil
	})
}

func contains(t *testing.T, text string, wants ...string) {
	t.Helper()
	for _, want := range wants {
		if !strings.Contains(text, want) {
			t.Fatalf("%q is not in:\n%s", want, text)
		}
	}
}

// Cost: one backend and a real device, a few seconds: the device installs the
// builtin package, and four turns run a shell job each.
func TestTheModelCreatesReadsEditsAndListsItsFilesWhereTheConversationWorks(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	alpha := b.Pair(master, "alpha")
	provider := b.Anthropic(master, vendor, "/alpha")
	b.CreateConversation(master, workFirst)
	home := alpha.Home()
	b.SwitchTo(master, workFirst, alpha, home)
	work := b.Open(master, vendor, workFirst, provider, "/alpha")

	heredoc := "mkdir src && cd src && demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF"
	created := work.Turn(backendtest.ShellCall("t1", heredoc, 10*time.Second), backendtest.Say("created"))
	contains(t, created.Received[0], "exitCode: 0", "Created notes.md")
	if got := readFile(t, filepath.Join(home, "src/notes.md")); got != "alpha\nbeta\ngamma\n" {
		t.Fatalf("notes.md holds %q", got)
	}
	// The model is offered the demi.builtin package's commands beside the
	// backend's own.
	system := string(backendtest.Marshal(backendtest.At(created.Requests[0], "system")))
	contains(t, system, "demi file create", "demi host")

	// The shell keeps its directory between turns.
	script := "pwd && demi file read notes.md | grep -n a | sort -r"
	read := work.Turn(backendtest.ShellCall("t2", script, 10*time.Second), backendtest.Say("read"))
	contains(t, read.Received[0], "/src\n3:gamma\n2:beta\n1:alpha")

	script = "demi file edit notes.md --old beta --new delta && cat notes.md"
	edited := work.Turn(backendtest.ShellCall("t3", script, 10*time.Second), backendtest.Say("edited"))
	contains(t, edited.Received[0], "Edited notes.md\nalpha\ndelta\ngamma")
	if got := readFile(t, filepath.Join(home, "src/notes.md")); got != "alpha\ndelta\ngamma\n" {
		t.Fatalf("notes.md holds %q", got)
	}

	listed := work.Turn(backendtest.ShellCall("t4", "ls && demi host current", 10*time.Second), backendtest.Say("listed"))
	contains(t, listed.Received[0], "notes.md", `host: machine "alpha"`)
	b.Stop()
}

func readFile(t *testing.T, path string) string {
	t.Helper()
	content, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(content)
}

// Cost: one backend and two real devices, a few seconds: each installs the
// builtin package, and five shell jobs run on them.
func TestASwitchMovesTheWorkAndTheDepartedDeviceKeepsItsFilesWithinReach(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	alpha := b.Pair(master, "alpha")
	beta := b.Pair(master, "beta")
	provider := b.Anthropic(master, vendor, "/work")
	b.CreateConversation(master, workFirst)
	onAlpha := directoryIn(t, alpha, "project")
	onBeta := directoryIn(t, beta, "project")
	b.SwitchTo(master, workFirst, alpha, onAlpha)
	work := b.Open(master, vendor, workFirst, provider, "/work")
	notes := "demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF"
	created := work.Turn(backendtest.ShellCall("t1", notes, 10*time.Second), backendtest.Say("created"))
	contains(t, created.Received[0], "Created notes.md")

	// On the other device, the next turn opens with the switch; the file stayed
	// where it was made.
	b.SwitchTo(master, workFirst, beta, onBeta)
	moved := work.Turn(
		backendtest.ShellCall("t2", "cat notes.md; echo exit=$?", 10*time.Second),
		backendtest.ShellCall("t3", notes, 10*time.Second),
		backendtest.Say("recreated"),
	)
	request := moved.FirstRequest()
	told := strings.Count(request, contextBlock)
	switches := strings.Count(request, targetSwitched)
	contains(t, request, "[Execution target switched]", `Previous target: the machine \"alpha\"`, `stays attached as \"alpha\"`)
	contains(t, moved.Received[0], "No such file or directory", "exit=1")
	contains(t, moved.Received[1], "Created notes.md")
	script := "demi file edit notes.md --old beta --new delta && cat notes.md"
	edited := work.Turn(backendtest.ShellCall("t4", script, 10*time.Second), backendtest.Say("edited"))
	contains(t, edited.Received[0], "alpha\ndelta\ngamma")
	if got := readFile(t, filepath.Join(onBeta, "notes.md")); got != "alpha\ndelta\ngamma\n" {
		t.Fatalf("beta's notes.md holds %q", got)
	}
	if got := readFile(t, filepath.Join(onAlpha, "notes.md")); got != "alpha\nbeta\ngamma\n" {
		t.Fatalf("alpha's notes.md holds %q", got)
	}
	// The model was told of the switch once: the history holds it, and the next
	// turn is told nothing new.
	if got := strings.Count(edited.FirstRequest(), contextBlock); got != told {
		t.Fatalf("the next turn is told %d context blocks, not %d", got, told)
	}

	// A change of the attached hosts alone is news of its own.
	renamed := b.Patch("/api/conversations/"+workFirst+"/hosts/"+alpha.ID(), master, backendtest.Map{"name": "first"})
	renamed.Expect(200)
	noted := work.Turn(backendtest.Say("noted")).FirstRequest()
	contains(t, noted, "[Attached hosts changed]", `\"first\" (online, shells start in`)
	if got := strings.Count(noted, targetSwitched); got != switches {
		t.Fatalf("the change of hosts repeats the switch: %d, not %d", got, switches)
	}

	// Back on the first device: the one left is attached under its name, and
	// demi host shell --host reaches it.
	b.SwitchTo(master, workFirst, alpha, onAlpha)
	script = `cat notes.md && demi host shell --host beta "cat notes.md"`
	back := work.Turn(backendtest.ShellCall("t5", script, 20*time.Second), backendtest.Say("back"))
	contains(t, back.FirstRequest(), "[Execution target switched]")
	contains(t, back.Received[0], "alpha\nbeta\ngamma\nalpha\ndelta\ngamma")
	b.Stop()
}

// Cost: one backend and a real device, a few seconds: the device installs the
// builtin package, and two conversations run two turns each on it at once.
func TestTwoConversationsOnOneDeviceKeepTheirDirectoriesShellsAndTodosApart(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	alpha := b.Pair(master, "alpha")
	home := alpha.Home()
	firstModel := b.Anthropic(master, vendor, "/a")
	secondModel := b.Anthropic(master, vendor, "/b")
	for _, id := range []string{workFirst, workSecond} {
		b.CreateConversation(master, id)
		b.SwitchTo(master, id, alpha, home)
	}
	a := b.Open(master, vendor, workFirst, firstModel, "/a")
	second := b.Open(master, vendor, workSecond, secondModel, "/b")

	// A moves into a directory, sets a variable and writes todos while B, at the
	// same time, does none of it.
	moving := `mkdir -p sub && cd sub && MARK=from-a && echo "a: $(pwd) $MARK" && demi todo add "draft the outline" && demi todo add "run the suite"`
	staying := `echo "b: $(pwd) mark=${MARK:-unset}"`
	bothTurns := func(aScript, bScript string, aAnswer, bAnswer string, id string) (backendtest.Turn, backendtest.Turn) {
		// Both turns are under way before either is waited for.
		beforeA := a.Start(backendtest.ShellCall("a"+id, aScript, 10*time.Second), backendtest.Say(aAnswer))
		beforeB := second.Start(backendtest.ShellCall("b"+id, bScript, 10*time.Second), backendtest.Say(bAnswer))
		a.Socket.UntilIdle()
		second.Socket.UntilIdle()
		return a.Observe(beforeA), second.Observe(beforeB)
	}
	first, other := bothTurns(moving, staying, "a moved", "b stayed", "1")
	sub := filepath.Join(home, "sub")
	contains(t, first.Received[0], fmt.Sprintf("a: %s from-a", sub))
	contains(t, other.Received[0], fmt.Sprintf("b: %s mark=unset", home))

	// A's shell carries its directory to the next turn and nothing else; B's
	// never moved. The todos last across turns, and stay with the conversation
	// that wrote them.
	todos := `echo "a: $(pwd) mark=${MARK:-unset}" && demi todo list --json`
	third, fourth := bothTurns(todos, staying+" && demi todo list --json", "a again", "b again", "2")
	contains(t, third.Received[0], fmt.Sprintf("a: %s mark=unset", sub), "draft the outline", "run the suite")
	contains(t, fourth.Received[0], fmt.Sprintf("b: %s mark=unset", home), `{"todos":[]}`)
	b.Stop()
}

// Cost: one backend and a real device, a few seconds: the device installs the
// builtin package, loses its runner in the middle of a job, and starts it again.
func TestARunnerLostInTheMiddleOfACommandEndsItAndTheReturnedRunnerServesTheNextTurn(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	alpha := b.Pair(master, "alpha")
	provider := b.Anthropic(master, vendor, "/alpha")
	b.CreateConversation(master, workFirst)
	home := alpha.Home()
	b.SwitchTo(master, workFirst, alpha, home)
	work := b.Open(master, vendor, workFirst, provider, "/alpha")
	written := work.Turn(backendtest.ShellCall("t1", "echo -n before > before.txt", 10*time.Second), backendtest.Say("written"))
	contains(t, written.Received[0], "exitCode: 0")

	// The command is running on the device when its runner goes.
	before := work.Start(
		backendtest.ShellCall("t2", "touch started; sleep 20; echo late", 30*time.Second),
		backendtest.Say("the runner is gone"),
	)
	untilExists(t, filepath.Join(home, "started"))
	alpha.Runner.Kill()
	work.Socket.UntilIdle()
	lost := work.Observe(before)
	contains(t, lost.Received[0], "exitCode: 127", "runner disconnected")

	if err := alpha.Runner.StartAgain(); err != nil {
		t.Fatal(err)
	}
	b.UntilOnline(master, alpha.ID(), true)
	back := work.Turn(backendtest.ShellCall("t3", "cat before.txt", 10*time.Second), backendtest.Say("back"))
	contains(t, back.Received[0], "before")
	b.Stop()
}

// keptFile is a file a command changed, as the transcript's shell view lists it.
type keptFile = map[string]any

// keptFiles returns each shell call's command and the files it kept, in the
// transcript's order.
func keptFiles(b *backendtest.Backend, master *backendtest.Session, id string) [][]keptFile {
	var calls [][]keptFile
	for _, block := range b.Transcript(master, id) {
		if backendtest.At(block, "type") != "tool_call" || backendtest.At(block, "view.kind") != "shell" {
			continue
		}
		var files []keptFile
		list, _ := backendtest.At(block, "view.files").([]any)
		for _, file := range list {
			files = append(files, file.(map[string]any))
		}
		calls = append(calls, files)
	}
	return calls
}

// editSides returns the two sides of the file's first edit segment, read from
// the blob route as the change view reads them.
func editSides(t *testing.T, b *backendtest.Backend, master *backendtest.Session, file keptFile) (string, string) {
	t.Helper()
	copies, _ := backendtest.At(file, "edits.0.copies").(map[string]any)
	if copies == nil {
		t.Fatalf("the edit has no copies: %v", file)
	}
	var sides []string
	for _, side := range []string{"original", "modified"} {
		sides = append(sides, b.Get("/api/blobs/"+copies[side].(string), master).Expect(200).Text())
	}
	return sides[0], sides[1]
}

func baseNames(files []keptFile) []string {
	var names []string
	for _, file := range files {
		names = append(names, filepath.Base(file["path"].(string)))
	}
	return names
}

// Cost: one backend and a real device, a few seconds: the device installs the
// builtin package and runs the commands whose edits are kept, and its runner
// stops.
func TestACommandsEditsAreKeptAsItsCallHistoryAndOutliveItsRunner(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	paired := b.Pair(master, "paired")
	provider := b.Anthropic(master, vendor, "/paired")
	b.CreateConversation(master, workFirst)
	b.SwitchTo(master, workFirst, paired, paired.Home())
	work := b.Open(master, vendor, workFirst, provider, "/paired")
	createFiles := "set -e\nprintf 'before\\n' > note.txt\nprintf 'created\\n' | demi file create native.txt\nprintf '\\0binary' > asset.bin\nprintf 'temporary' > removed.txt\nrm removed.txt"
	work.Turn(backendtest.ShellCall("create", createFiles, 10*time.Second), backendtest.Say("Created the files."))
	first := keptFiles(b, master, workFirst)[0]
	if names := baseNames(first); !slices.Equal(names, []string{"note.txt", "native.txt", "asset.bin"}) {
		t.Fatalf("the kept files are %v", names)
	}
	// A binary file's edit is listed without copies.
	if backendtest.At(first[2], "edits.0.copies") != nil {
		t.Fatalf("a binary file's edit has copies: %v", first[2])
	}
	if original, modified := editSides(t, b, master, first[0]); original != "" || modified != "before\n" {
		t.Fatalf("note.txt's sides are %q and %q", original, modified)
	}

	patch := "demi file patch <<'PATCH'\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-before\n+after\n--- a/native.txt\n+++ b/native.txt\n@@ -1 +1 @@\n-created\n+patched\nPATCH"
	work.Turn(backendtest.ShellCall("patch", patch, 10*time.Second), backendtest.Say("Patched both files."))
	calls := keptFiles(b, master, workFirst)
	second := calls[len(calls)-1]
	var kinds []string
	for _, file := range second {
		kinds = append(kinds, file["kind"].(string))
	}
	if !slices.Equal(kinds, []string{"modified", "modified"}) {
		t.Fatalf("the patch's kinds are %v", kinds)
	}
	if original, modified := editSides(t, b, master, second[0]); original != "before\n" || modified != "after\n" {
		t.Fatalf("note.txt's sides are %q and %q", original, modified)
	}

	// Neither the archive nor the runner's absence takes them away.
	b.Patch("/api/conversations/"+workFirst, master, backendtest.Map{"archived": true}).Expect(200)
	paired.Runner.Kill()
	if original, modified := editSides(t, b, master, second[1]); original != "created\n" || modified != "patched\n" {
		t.Fatalf("native.txt's sides are %q and %q", original, modified)
	}
	b.Stop()
}

// Cost: one backend started twice and a real device, a few seconds: the device
// installs the builtin package, and its runner comes back after the backend's
// restart to run the next job.
func TestAfterABackendRestartTheRunnerComesBackAndTheConversationGoesOnThere(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin())
	b, master := h.StartSetUp()
	alpha := b.Pair(master, "alpha")
	provider := b.Anthropic(master, vendor, "/alpha")
	b.CreateConversation(master, workFirst)
	home := alpha.Home()
	b.SwitchTo(master, workFirst, alpha, home)
	work := b.Open(master, vendor, workFirst, provider, "/alpha")
	kept := work.Turn(backendtest.ShellCall("t1", "echo -n kept > kept.txt && cat kept.txt", 10*time.Second), backendtest.Say("remember me"))
	contains(t, kept.Received[0], "kept")
	// The conversation's first command is command 1.
	if id := backendtest.Field(t, kept.Received[0], "commandId"); id != "1" {
		t.Fatalf("the first command is %s", id)
	}

	// The backend stops under a running command: the call ends as an error and
	// the turn as interrupted, with nothing left dangling.
	work.Start(backendtest.ShellCall("t2", "touch started; sleep 20", 30*time.Second))
	untilExists(t, filepath.Join(home, "started"))
	// The backend comes back at its address, where the runner reconnects.
	b.Stop()
	b = h.Start()
	b.UntilOnline(master, alpha.ID(), true)
	blocks := b.Transcript(master, workFirst)
	kinds := backendtest.BlockKinds(blocks)
	turn := slices.Index(kinds, "user")
	for index, kind := range kinds {
		if kind == "user" {
			turn = index
		}
	}
	if !slices.Equal(kinds[turn:], []string{"user", "tool_call", "response", "error"}) {
		t.Fatalf("the interrupted turn is %v", kinds[turn:])
	}
	if status := backendtest.At(blocks[turn+1], "status"); status != "error" {
		t.Fatalf("the cut call is %v: %v", status, blocks[turn+1])
	}
	if code := backendtest.At(blocks[len(blocks)-1], "code"); code != "interrupted" {
		t.Fatalf("the turn ends as %v", code)
	}

	// The runner came back on its own, and the next turn runs there.
	work.Reconnect(b, master, workFirst, provider)
	script := "cat kept.txt && demi host current"
	after := work.Turn(backendtest.ShellCall("t3", script, 10*time.Second), backendtest.Say("and again"))
	contains(t, after.Received[0], "Tool call aborted")
	contains(t, after.Received[1], "kept", `host: machine "alpha"`)
	// The cut command was 2; the restarted backend gives no number twice.
	if id := backendtest.Field(t, after.Received[1], "commandId"); id != "3" {
		t.Fatalf("the next command is %s", id)
	}
	b.Stop()
}

// movedFromAlphaToBeta makes a conversation that worked on alpha and then moved
// to beta, which left alpha attached.
func movedFromAlphaToBeta(t *testing.T, b *backendtest.Backend, master *backendtest.Session, alpha, beta *backendtest.Paired) (string, string) {
	t.Helper()
	b.CreateConversation(master, workFirst)
	onAlpha, onBeta := alpha.Home(), beta.Home()
	b.SwitchTo(master, workFirst, alpha, onAlpha)
	b.SwitchTo(master, workFirst, beta, onBeta)
	return onAlpha, onBeta
}

// Cost: one backend and two real devices, several seconds: each installs the
// builtin package, and demi host shell runs jobs on both.
func TestDemiHostShellCarriesBytesBothWaysThroughPipesAndKeepsTheFarHostsDirectory(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	alpha := b.Pair(master, "alpha")
	beta := b.Pair(master, "beta")
	provider := b.Anthropic(master, vendor, "/work")
	a, bDir := movedFromAlphaToBeta(t, b, master, alpha, beta)
	// Well past the view a job's output travels in: only a pipe carries it
	// whole.
	payload := make([]byte, 300*1024)
	for index := range payload {
		payload[index] = byte(index * 31 % 256)
	}
	if err := os.WriteFile(filepath.Join(a, "notes.bin"), payload, 0o644); err != nil {
		t.Fatal(err)
	}
	work := b.Open(master, vendor, workFirst, provider, "/work")
	aID := alpha.ID()

	pull := fmt.Sprintf(`demi host list && demi host shell --host alpha "tar c -C %s notes.bin" | tar x && cmp notes.bin %s/notes.bin && echo copied`, a, a)
	pulled := work.Turn(backendtest.ShellCall("t1", pull, 30*time.Second), backendtest.Say("one"))
	result := pulled.Received[0]
	contains(t, result,
		fmt.Sprintf("beta  %s  online  %s  (main)", beta.ID(), bDir),
		fmt.Sprintf("alpha  %s  online  %s  (attached)", aID, a),
		"copied",
	)
	if got, err := os.ReadFile(filepath.Join(bDir, "notes.bin")); err != nil || string(got) != string(payload) {
		t.Fatalf("the pulled file differs: %v", err)
	}

	// The other way: the caller's pipe is the far job's standard input.
	push := fmt.Sprintf(`head -c 250000 notes.bin > push.bin && tar c push.bin | demi host shell --host alpha "tar x -C %s" && cmp push.bin %s/push.bin && echo pushed`, a, a)
	pushed := work.Turn(backendtest.ShellCall("t2", push, 30*time.Second), backendtest.Say("two"))
	contains(t, pushed.Received[0], "pushed")
	if got, err := os.ReadFile(filepath.Join(a, "push.bin")); err != nil || string(got) != string(payload[:250_000]) {
		t.Fatalf("the pushed file differs: %v", err)
	}

	// Where a shell on the attached host ends is where the next one starts, and
	// --host takes the device's id as well.
	wander := fmt.Sprintf(`demi host shell --host alpha "mkdir -p sub && cd sub && pwd" && demi host shell --host %s "pwd" && demi host list`, aID)
	wandered := work.Turn(backendtest.ShellCall("t3", wander, 30*time.Second), backendtest.Say("three"))
	sub := a + "/sub"
	contains(t, wandered.Received[0], sub+"\n"+sub+"\n", fmt.Sprintf("alpha  %s  online  %s  (attached)", aID, sub))

	stranger := work.Turn(backendtest.ShellCall("t4", `demi host shell --host nope "echo hi"; echo exit=$?`, 10*time.Second), backendtest.Say("four"))
	contains(t, stranger.Received[0], "host nope is not reachable", "exit=1")
	b.Stop()
}

// Cost: one backend and two real devices, several seconds: each installs the
// builtin package, and the far job runs through demi host shell; the reads
// that wait for its first output are 400 ms apart.
func TestDemiHostShellShowsTheFarJobsErrorsAsTheyComeTakesItsInputAndIsStoppedWithIt(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	alpha := b.Pair(master, "alpha")
	beta := b.Pair(master, "beta")
	provider := b.Anthropic(master, vendor, "/work")
	a, _ := movedFromAlphaToBeta(t, b, master, alpha, beta)
	work := b.Open(master, vendor, workFirst, provider, "/work")

	// The far job reports on standard error at once and waits for a line, then
	// runs a process of its own; the model's window ends while it runs.
	far := `printf "ready\n" >&2; read line; printf "%s" "$line" > got.txt; sh -c "echo \$\$ > far.pid; exec /bin/sleep 30"`
	script := "demi host shell --host alpha '" + far + "'"
	started := work.Turn(backendtest.ShellCall("t1", script, 500*time.Millisecond), backendtest.Say("waiting"))
	result := started.Received[0]
	if !strings.HasPrefix(result, "status: running") {
		t.Fatalf("the far job's first result is %s", result)
	}
	command := backendtest.Field(t, result, "commandId")
	// The far job's error output reaches the model while the job runs: the model
	// reads the command's output, each read showing what came since the one
	// before, until ready is there. The far job's start (a login shell on alpha)
	// may outlast the window above, and its shell's printf writes a byte at a
	// time, so ready may come split between reads.
	output := backendtest.ShownOutput(result)
	deadline := time.Now().Add(backendtest.Patience)
	for reads := 1; !strings.Contains(output, "ready"); reads++ {
		if time.Now().After(deadline) {
			t.Fatalf("the far job's ready never came: %q", output)
		}
		// Each read is a turn of two requests, and the backend starts at most 120
		// a minute (usage-and-quota.md § Rate limit): 400 ms apart, the reads of
		// the whole wait make at most 100.
		time.Sleep(400 * time.Millisecond)
		read := work.Turn(scripted.ToolUse("s"+strconv.Itoa(reads), "shell_status", backendtest.Map{"commandId": command}), backendtest.Say("still waiting"))
		if len(read.Received) == 0 {
			t.Fatal("the read does not reach the model")
		}
		if !strings.HasPrefix(read.Received[0], "status: running") {
			t.Fatalf("the far job stopped: %s", read.Received[0])
		}
		output += backendtest.ShownOutput(read.Received[0])
	}

	work.Turn(scripted.ToolUse("t2", "shell_write", backendtest.Map{"commandId": command, "stdin": "hello\n"}), backendtest.Say("fed"))
	got := filepath.Join(a, "got.txt")
	pidFile := filepath.Join(a, "far.pid")
	backendtest.Eventually(t, "the far job read the line and went on", func() bool {
		line, _ := os.ReadFile(got)
		pid, _ := os.ReadFile(pidFile)
		return string(line) == "hello" && strings.HasSuffix(string(pid), "\n")
	})

	// Stopping the command stops the far job with it.
	pid, err := strconv.Atoi(strings.TrimSpace(readFile(t, pidFile)))
	if err != nil {
		t.Fatal(err)
	}
	alive := func() bool { return syscall.Kill(pid, 0) == nil }
	if !alive() {
		t.Fatal("the far job is not running")
	}
	stopped := work.Turn(scripted.ToolUse("t3", "shell_abort", backendtest.Map{"commandId": command}), backendtest.Say("stopped"))
	if !strings.HasPrefix(stopped.Received[0], "status: aborted") {
		t.Fatalf("the abort answers %s", stopped.Received[0])
	}
	backendtest.Eventually(t, "the far job ended", func() bool { return !alive() })
	b.Stop()
}

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

// containsText reports whether the text holds the part.
func containsText(text, part string) bool {
	return strings.Contains(text, part)
}

// cloneMap returns the map with one member set.
func cloneMap(source backendtest.Map, key string, value any) backendtest.Map {
	copied := backendtest.Map{}
	for name, member := range source {
		copied[name] = member
	}
	copied[key] = value
	return copied
}
