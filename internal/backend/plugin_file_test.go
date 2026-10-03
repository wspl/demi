package backend_test

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/provider/providertest"
)

// Real runner and native file service; each script is one tool call in the same turn.
func TestPluginFileReadsCreatesInsideAndOutsideWorkspace(t *testing.T) {
	t.Parallel()
	scripts := []string{
		"demi file create note.txt <<'EOF'\nhello world\nEOF",
		"demi file read note.txt",
		"demi file create note.txt <<'EOF'\nagain\nEOF",
		"demi file create src/foo.txt <<'EOF'\nhello\nEOF\ncat src/foo.txt",
		"demi file create \"$(cd .. && pwd)/absolute.txt\" <<'EOF'\nnope\nEOF",
		"demi file create ../relative.txt <<'EOF'\nnope\nEOF",
		"demi file read shot.png",
		"demi file read shot.png | wc -c",
		"demi --help",
	}
	var turns []providertest.Turn
	for i, script := range scripts {
		turns = append(turns, providertest.Events(filesExec(t, fmt.Sprintf("f%d", i), script, 10000)))
	}
	turns = append(turns, providertest.Events(providertest.Text("done"), providertest.Response(1, 1)))
	runtime := providertest.NewScriptedRuntime(t, turns...)
	w := filesScripted(t, runtime)
	wireMust(t, os.WriteFile(filepath.Join(w.root, "shot.png"), []byte{0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 255, 254}, 0644))
	_, err := w.socket.Chat(w.ctx, "message-1", "Work on the files.")
	wireMust(t, err)
	requests := runtime.Requests()
	results := make([]string, len(scripts))
	for i := range scripts {
		results[i] = filesResult(t, requests[i+1])
	}
	exit := func(i int, code string) {
		t.Helper()
		if !strings.HasPrefix(results[i], "status: exited\n") {
			t.Fatal(results[i])
		}
		conversationEqual(t, toolstest.Field(results[i], "exitCode"), code)
	}
	shown := func(i int) string { return toolstest.ShownOutput(results[i]) }
	exit(0, "0")
	conversationEqual(t, shown(0), "Created note.txt\n")
	conversationEqual(t, shown(1), "hello world\n")
	exit(2, "1")
	conversationEqual(t, shown(3), "Created src/foo.txt\nhello\n")
	exit(4, "0")
	exit(5, "0")
	exit(6, "0")
	filesContains(t, shown(6), "<binary stdout: 11 bytes>\n", "save it: demi shell output ")
	conversationEqual(t, strings.TrimSpace(shown(7)), "11")
	filesContains(t, shown(8), "demi file create", `Success output: writes "Created <path>" to stdout`, "shown to you as viewable media")
	conversationEqual(t, filesRead(t, filepath.Join(w.root, "note.txt")), "hello world\n")
	for _, name := range []string{"absolute.txt", "relative.txt"} {
		conversationEqual(t, filesRead(t, filepath.Join(w.paired.Runner.Home(), name)), "nope\n")
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.b.Close(w.ctx))
}

// Real runner and native file service; each script is one tool call in the same turn.
func TestPluginFileEditsAndPatchesAtomically(t *testing.T) {
	t.Parallel()
	scripts := []string{
		"demi file edit file.txt --old two --new changed",
		"demi file edit file.txt --old two --new changed --occurrence 2 && cat file.txt",
		"demi file edit context.txt --old target --new changed --context 2",
		"cat context.txt && demi file edit context.txt --old target --new changed --context 3 && cat context.txt",
		"demi file edit empty-old.txt --old \"\" --new changed",
		"demi file create patch.txt <<'EOF'\none\ntwo\nEOF\ndemi file patch <<'PATCH' && cat patch.txt\n--- a/patch.txt\n+++ b/patch.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+three\nPATCH",
		"demi file create timed.txt <<'EOF'\nold\nEOF\ndemi file patch <<'PATCH' && cat timed.txt\n--- a/timed.txt 2026-06-17 00:00:00.000000000 +0800\n+++ b/timed.txt 2026-06-17 00:00:01.000000000 +0800\n@@ -1 +1 @@\n-old\n+new\nPATCH",
		"demi file create existing.txt <<'EOF'\none\nEOF\ndemi file patch <<'PATCH' && cat existing.txt nested/new.txt\n--- a/existing.txt\n+++ b/existing.txt\n@@ -1 +1 @@\n-one\n+changed\n--- /dev/null\n+++ b/nested/new.txt\n@@ -0,0 +1,2 @@\n+new\n+file\nPATCH",
		"demi file create doomed.txt <<'EOF'\nremove\nEOF\ndemi file patch <<'PATCH' && test ! -e doomed.txt && echo gone\n--- a/doomed.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-remove\nPATCH",
		"demi file patch <<'PATCH'\n--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-wrong\n+changed\nPATCH",
		"demi file create inside.txt <<'EOF'\ninside\nEOF\noutside=\"$(cd .. && pwd)/outside.txt\"\ndemi file patch <<PATCH && cat inside.txt\n--- a/inside.txt\n+++ b/inside.txt\n@@ -1 +1 @@\n-inside\n+changed\n--- /dev/null\n+++ $outside\n@@ -0,0 +1 @@\n+outside\nPATCH",
	}
	var turns []providertest.Turn
	for i, script := range scripts {
		turns = append(turns, providertest.Events(filesExec(t, fmt.Sprintf("f%d", i), script, 10000)))
	}
	turns = append(turns, providertest.Events(providertest.Text("done"), providertest.Response(1, 1)))
	runtime := providertest.NewScriptedRuntime(t, turns...)
	w := filesScripted(t, runtime)
	for name, content := range map[string]string{"file.txt": "one\ntwo\ntwo\n", "context.txt": "target\nmiddle\ntarget\n", "empty-old.txt": "content\n", "first.txt": "first\n", "second.txt": "second\n"} {
		wireMust(t, os.WriteFile(filepath.Join(w.root, name), []byte(content), 0644))
	}
	_, err := w.socket.Chat(w.ctx, "message-1", "Work on the files.")
	wireMust(t, err)
	requests := runtime.Requests()
	results := make([]string, len(scripts))
	for i := range scripts {
		results[i] = filesResult(t, requests[i+1])
	}
	exit := func(i int, code string) {
		t.Helper()
		if !strings.HasPrefix(results[i], "status: exited\n") {
			t.Fatal(results[i])
		}
		conversationEqual(t, toolstest.Field(results[i], "exitCode"), code)
	}
	shown := func(i int) string { return toolstest.ShownOutput(results[i]) }
	exit(0, "1")
	filesContains(t, shown(0), "Multiple matches in file.txt; specify --occurrence or --context")
	conversationEqual(t, shown(1), "Edited file.txt\none\ntwo\nchanged\n")
	exit(2, "1")
	filesContains(t, shown(2), "Context line 2 is ambiguous", "occurrence 1 at line 1", "occurrence 2 at line 3")
	conversationEqual(t, shown(3), "target\nmiddle\ntarget\nEdited context.txt\ntarget\nmiddle\nchanged\n")
	exit(4, "1")
	filesContains(t, shown(4), `Invalid command arguments: "old" is shorter than 1 character`)
	conversationEqual(t, filesRead(t, filepath.Join(w.root, "empty-old.txt")), "content\n")
	conversationEqual(t, shown(5), "Created patch.txt\nPatched 1 file(s)\none\nthree\n")
	conversationEqual(t, shown(6), "Created timed.txt\nPatched 1 file(s)\nnew\n")
	conversationEqual(t, shown(7), "Created existing.txt\nPatched 2 file(s)\nchanged\nnew\nfile\n")
	conversationEqual(t, shown(8), "Created doomed.txt\nPatched 1 file(s)\ngone\n")
	exit(9, "1")
	filesContains(t, shown(9), "Patch does not apply to second.txt")
	conversationEqual(t, filesRead(t, filepath.Join(w.root, "first.txt")), "first\n")
	conversationEqual(t, shown(10), "Created inside.txt\nPatched 2 file(s)\nchanged\n")
	conversationEqual(t, filesRead(t, filepath.Join(w.paired.Runner.Home(), "outside.txt")), "outside\n")
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.b.Close(w.ctx))
}
