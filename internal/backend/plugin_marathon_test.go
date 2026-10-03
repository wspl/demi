package backend_test

import (
	"fmt"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

// filesScripted selects the Rust scenario's in-process provider on the shared page driver.
func filesScripted(t *testing.T, script *providertest.ScriptedRuntime) filesWork {
	t.Helper()
	w := filesWorking(t, "demi-file", func(h *backendtest.Harness) {
		h.Config.Clock = core.SystemClock{}
		h.Config.Families.Register("files-script", conversationFamily{build: func(providers.FamilyArgs) provider.Runtime { return script }})
	})
	filesCloseTree(w.ctx, t, w.socket)
	body := `{"source":"custom","providerType":"files-script","label":"Script","apiKey":"k","models":[` + conversationConfigured(8000) + `]}`
	entry := conversationDecode(t, conversationRequest(w.ctx, t, w.b, &w.s, "POST", "/api/providers", body, 201), webapi.DecodeProviderAnswer)
	conversationChoose(w.ctx, t, w.b, &w.s, filesConversation, string(entry.Provider.ID), "m")
	_, err := w.socket.Open(w.ctx)
	wireMust(t, err)
	return w
}

// filesResult reads the last tool result in the scripted model's request.
func filesResult(t *testing.T, r provider.InferenceRequest) string {
	t.Helper()
	for i := len(r.Items) - 1; i >= 0; i-- {
		if v, ok := r.Items[i].(*provider.ToolResult); ok {
			var text []string
			for _, p := range v.Output {
				if part, ok := p.(*provider.TextPart); ok {
					text = append(text, part.Text)
				}
			}
			return strings.Join(text, "\n")
		}
	}
	t.Error("request has no text tool result")
	return ""
}

// filesExec composes a shell invocation using the shared contract JSON encoder.
func filesExec(t *testing.T, id, script string, timeout int) provider.Event {
	t.Helper()
	input, err := contract.EncodeObject([]contract.Field{{Name: "script", Value: script}, {Name: "timeoutMs", Value: timeout}})
	wireMust(t, err)
	return providertest.ToolCall(id, "shell_exec", input)
}

// Six scripts run on a real runner, preserving the todo and shell across messages.
func TestPluginMarathonCodingWorkflowKeepsFilesTodosAndShell(t *testing.T) {
	t.Parallel()
	scripts := []string{"demi file create src/app.ts <<'EOF'\nexport const value = 1\nEOF", `demi todo add "Run tests" --json`, "grep -q 'value = 2' src/app.ts", `demi file edit src/app.ts --old "1" --new "2" && cd src`, "grep -q 'value = 2' app.ts && echo passed", "demi todo done T1 && pwd"}
	var turns []providertest.Turn
	for i, script := range scripts {
		turns = append(turns, providertest.Events(filesExec(t, fmt.Sprintf("t%d", i), script, 30000)))
		if i == 4 || i == 5 {
			turns = append(turns, providertest.Events(treeSay("done")...))
		}
	}
	runtime := providertest.NewScriptedRuntime(t, turns...)
	w := filesScripted(t, runtime)
	_, err := w.socket.Chat(w.ctx, "message-1", "Create the app file, track its test, then fix the value.")
	wireMust(t, err)
	requests := runtime.Requests()
	results := make([]string, 5)
	for i := range results {
		results[i] = filesResult(t, requests[i+1])
	}
	conversationEqual(t, len(requests), 6)
	for _, result := range results {
		if !strings.HasPrefix(result, "status: exited\n") {
			t.Fatal(result)
		}
	}
	conversationEqual(t, toolstest.Field(results[0], "exitCode"), "0")
	conversationEqual(t, toolstest.ShownOutput(results[0]), "Created src/app.ts\n")
	conversationEqual(t, strings.TrimSpace(toolstest.ShownOutput(results[1])), `{"todo":{"id":"T1","text":"Run tests","status":"pending"}}`)
	conversationEqual(t, toolstest.Field(results[2], "exitCode"), "1")
	conversationEqual(t, toolstest.ShownOutput(results[3]), "Edited src/app.ts\n")
	conversationEqual(t, toolstest.ShownOutput(results[4]), "passed\n")
	blocks := conversationTranscript(w.ctx, t, w.b, &w.s, filesConversation).Blocks
	conversationEqual(t, conversationKinds(t, blocks), []string{"user", "tool_call", "tool_call", "tool_call", "tool_call", "tool_call", "text", "response"})
	for _, block := range blocks {
		if c, ok := block.(*core.ToolCallBlock); ok {
			conversationEqual(t, string(c.Status), "completed")
		}
	}
	released := core.CommandID(toolstest.Field(results[0], "commandId"))
	wireMust(t, w.socket.Send(w.ctx, &framewire.ShellAbortFrame{CommandID: released}))
	frame, err := w.socket.Next(w.ctx)
	wireMust(t, err)
	if _, ok := frame.(*framewire.ErrorFrame); !ok {
		t.Fatalf("released command: %T", frame)
	}
	_, err = w.socket.Chat(w.ctx, "message-2", "Mark the test done.")
	wireMust(t, err)
	lastResult := filesResult(t, runtime.Requests()[7])
	conversationEqual(t, toolstest.ShownOutput(lastResult), "[x] T1 Run tests\n"+w.root+"/src\n")
	conversationEqual(t, filesRead(t, filepath.Join(w.root, "src/app.ts")), "export const value = 2\n")
	first := runtime.Requests()[0]
	var names []string
	for _, tool := range first.Tools {
		names = append(names, tool.Name)
	}
	conversationEqual(t, names, []string{"shell_exec", "shell_status", "shell_write", "shell_abort", "yield"})
	filesContains(t, first.SystemPrompt, "You are a coding agent.", "Registered commands:", "demi file create", "demi todo update <id> [--text <text>] [--status <pending|in_progress|done>] [--json]", "demi agent spawn", "demi agent abort", "demi agent list", "demi agent show", "demi agent send <id> [--json] <<'EOF'", "Stdin body: content", "Stdin body: patch", "Stdin body: prompt", "Stdin body: message", "cannot see this conversation", "State the exact shape of the last assistant text it should return.", "Available: none")
	for _, absent := range []string{"demi agent steer", "--content", "--patch", "--prompt", "--message"} {
		if strings.Contains(first.SystemPrompt, absent) {
			t.Fatalf("prompt contains %s", absent)
		}
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.b.Close(w.ctx))
}

// A scripted model feeds a real reader, yields between checks, and aborts a long job.
func TestPluginMarathonShellToolsFeedStopAndYield(t *testing.T) {
	t.Parallel()
	var mu sync.Mutex
	phase := 0
	var reader, long core.CommandID
	var output string
	var seen []string
	polls := 0
	call := func(name, input string) provider.Event {
		polls++
		return providertest.ToolCall(fmt.Sprintf("%s-%d", name, polls), name, []byte(input))
	}
	turns := []providertest.Turn{providertest.Events(filesExec(t, "reader", `read name; echo "hello $name"`, 200))}
	for range 100 {
		turns = append(turns, providertest.Respond(func(r provider.InferenceRequest) []provider.Event {
			mu.Lock()
			defer mu.Unlock()
			result := filesResult(t, r)
			seen = append(seen, result)
			switch phase {
			case 0:
				reader = core.CommandID(toolstest.Field(result, "commandId"))
				phase = 1
				return []provider.Event{call("shell_status", `{"commandId":"`+string(reader)+`"}`)}
			case 1:
				phase = 2
				return []provider.Event{call("shell_write", `{"commandId":"`+string(reader)+`","stdin":"Alice\n","description":"Name given"}`)}
			case 2:
				if strings.HasPrefix(result, "yield scheduled") {
					return []provider.Event{call("shell_status", `{"commandId":"`+string(reader)+`"}`)}
				}
				output += toolstest.ShownOutput(result)
				if toolstest.Field(result, "status") == "running" {
					return []provider.Event{call("yield", `{"durationMs":20}`)}
				}
				phase = 3
				return []provider.Event{filesExec(t, "long", "echo long-ready; sleep 30", 200)}
			case 3:
				long = core.CommandID(toolstest.Field(result, "commandId"))
				phase = 4
				return []provider.Event{call("shell_abort", `{"commandId":"`+string(long)+`"}`)}
			case 4:
				phase = 5
				return []provider.Event{providertest.Text("stopped"), providertest.Response(1, 1)}
			default:
				t.Error("model asked after its end")
				return nil
			}
		}))
	}
	w := filesScripted(t, providertest.NewScriptedRuntime(t, turns...))
	wireMust(t, w.socket.Send(w.ctx, backendtest.ConversationText("message-1", "Greet Alice, then run and stop the long command.")))
	var frames []framewire.ServerFrame
	for {
		next, err := w.socket.UntilIdle(w.ctx)
		wireMust(t, err)
		frames = append(frames, next...)
		mu.Lock()
		done := phase == 5
		mu.Unlock()
		if done {
			break
		}
	}
	mu.Lock()
	recorded := append([]string(nil), seen...)
	readerID, longID, readerOutput := reader, long, output
	mu.Unlock()
	for _, r := range recorded[:2] {
		if !strings.HasPrefix(r, "status: running\n") {
			t.Fatal(r)
		}
	}
	conversationEqual(t, readerOutput, "hello Alice\n")
	aborted := recorded[len(recorded)-1]
	if !strings.HasPrefix(aborted, "status: aborted\n") {
		t.Fatal(aborted)
	}
	filesContains(t, aborted, "next: command was intentionally stopped.")
	var ends []framewire.ShellStatus
	ended := map[core.CommandID]bool{}
	for _, f := range frames {
		if v, ok := f.(*framewire.ShellOutputFrame); ok {
			id := v.Status.Command().CommandID
			if ended[id] {
				t.Fatalf("frame after end of %s", id)
			}
			if filesEnded(v.Status) {
				ended[id] = true
				ends = append(ends, v.Status)
			}
		}
	}
	conversationEqual(t, len(ends), 2)
	conversationEqual(t, []core.CommandID{ends[0].Command().CommandID, ends[1].Command().CommandID}, []core.CommandID{readerID, longID})
	if greeted, ok := ends[0].(*framewire.ExitedStatus); !ok || greeted.ExitCode != 0 || greeted.Tail != "hello Alice\n" {
		t.Fatalf("reader: %+v", ends[0])
	}
	if _, ok := ends[1].(*framewire.AbortedStatus); !ok {
		t.Fatalf("long: %+v", ends[1])
	}
	for _, b := range conversationTranscript(w.ctx, t, w.b, &w.s, filesConversation).Blocks {
		if call, ok := b.(*core.ToolCallBlock); ok && string(call.Status) == "error" {
			t.Fatalf("failed call: %+v", call)
		}
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.b.Close(w.ctx))
}
