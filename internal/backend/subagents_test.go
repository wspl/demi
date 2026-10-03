package backend_test

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

// treeScript assigns one sequence to each root and each child in arrival order.
type treeScript struct {
	t         *testing.T
	mu        sync.Mutex
	roots     map[string][][]provider.Event
	children  map[string][][]provider.Event
	unclaimed [][][]provider.Event
	asked     []provider.InferenceRequest
}

func (s *treeScript) root(id string, answers ...[]provider.Event) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.roots[id] = append(s.roots[id], answers...)
}
func (s *treeScript) child(answers ...[]provider.Event) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.unclaimed = append(s.unclaimed, answers)
}
func (s *treeScript) requests(id string, root bool) []provider.InferenceRequest {
	s.mu.Lock()
	defer s.mu.Unlock()
	var out []provider.InferenceRequest
	for _, r := range s.asked {
		if (r.SessionID == id) == root {
			out = append(out, r)
		}
	}
	return out
}
func (s *treeScript) Run(ctx context.Context, r provider.InferenceRequest) provider.Run {
	s.mu.Lock()
	s.asked = append(s.asked, r)
	scripts := s.roots
	if _, ok := scripts[r.SessionID]; !ok {
		scripts = s.children
		if _, ok := scripts[r.SessionID]; !ok && len(s.unclaimed) > 0 {
			scripts[r.SessionID] = s.unclaimed[0]
			s.unclaimed = s.unclaimed[1:]
		}
	}
	answers := scripts[r.SessionID]
	if len(answers) == 0 {
		s.mu.Unlock()
		s.t.Errorf("no answer scripted for session %s", r.SessionID)
		return providertest.Events()(ctx, r)
	}
	answer := answers[0]
	scripts[r.SessionID] = answers[1:]
	s.mu.Unlock()
	return providertest.Events(answer...)(ctx, r)
}
func (s *treeScript) Fresh() provider.Runtime                       { return s }
func (*treeScript) Close(context.Context) error                     { return nil }
func (*treeScript) RequestLimits(core.Model) provider.RequestLimits { return provider.RequestLimits{} }

// treeShell scripts a tool request separately for one node's model.
func treeShell(t *testing.T, id, script string) []provider.Event {
	t.Helper()
	input := fmt.Sprintf(`{"description":%s,"script":%s,"timeoutMs":60000}`, conversationJSON(t, id), conversationJSON(t, script))
	return []provider.Event{providertest.ToolCall(id, "shell_exec", json.RawMessage(input)), providertest.Response(1, 1)}
}
func treeSay(text string) []provider.Event {
	return []provider.Event{providertest.Text(text), providertest.Response(1, 1)}
}

// conversationTree starts a node-scripted family on a real paired target.
func conversationTree(t *testing.T) (context.Context, *backendtest.TestBackend, backendtest.Session, *treeScript, string) {
	t.Helper()
	ctx, h := conversationHarness(t)
	script := &treeScript{t: t, roots: make(map[string][][]provider.Event), children: make(map[string][][]provider.Event)}
	h.Config.Families.Register("tree", conversationFamily{build: func(providers.FamilyArgs) provider.Runtime { return script }})
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	body := `{"source":"custom","providerType":"tree","label":"Tree","apiKey":"k","models":[{"id":"m","displayName":"M","contextWindow":100000,"outputLimit":null,"thinkingEfforts":[],"acceptedExtensions":null,"fastTier":null}]}`
	entry := conversationDecode(t, conversationRequest(ctx, t, b, &s, "POST", "/api/providers", body, 201), webapi.DecodeProviderAnswer)
	conversationCreate(ctx, t, b, &s, conversationFirst)
	conversationChoose(ctx, t, b, &s, conversationFirst, string(entry.Provider.ID), "m")
	_, root := conversationOnDevice(ctx, t, h, b, &s, conversationFirst)
	return ctx, b, s, script, root
}

// treeRequestText observes typed model input, including tool-result text.
func treeRequestText(r provider.InferenceRequest) string {
	var text strings.Builder
	for _, item := range r.Items {
		if result, ok := item.(*provider.ToolResult); ok {
			for _, part := range result.Output {
				if p, ok := part.(*provider.TextPart); ok {
					text.WriteString(p.Text)
				}
			}
		}
	}
	return text.String()
}

// A real child works past its spawn job; the test releases its file wait.
func TestChildSharesFilesButKeepsOwnTodosAfterSpawn(t *testing.T) {
	ctx, b, s, scripts, root := conversationTree(t)
	socket := conversationOpen(ctx, t, b, &s, conversationFirst)
	scripts.root(conversationFirst, treeShell(t, "t1", `printf 'the answer is 42\n' > notes.md && demi todo add root-only`), treeSay("written"))
	_, err := socket.Chat(ctx, "m1", "Write the notes")
	wireMust(t, err)
	child := `until [ -f go ]; do sleep 0.05; done; cat notes.md && printf 'from the child\n' > reply.md && demi host shell --host laptop "demi todo add child-only" && demi todo list`
	scripts.child(treeShell(t, "c1", child), treeSay("the file says 42"))
	scripts.root(conversationFirst, treeShell(t, "t2", `demi agent spawn --description reader <<< 'Read notes.md and answer'`), treeSay("dispatched"), treeSay("received"))
	_, err = socket.Chat(ctx, "m2", "Delegate the reading")
	wireMust(t, err)
	wireMust(t, os.WriteFile(filepath.Join(root, "go"), nil, 0644))
	_, err = socket.UntilIdle(ctx)
	wireMust(t, err)
	children := scripts.requests(conversationFirst, false)
	read := treeRequestText(children[len(children)-1])
	if !strings.Contains(read, "the answer is 42") || !strings.Contains(read, "child-only") || strings.Contains(read, "root-only") {
		t.Fatalf("child files/todos: %s", read)
	}
	bytes, err := os.ReadFile(filepath.Join(root, "reply.md"))
	wireMust(t, err)
	conversationEqual(t, string(bytes), "from the child\n")
	scripts.root(conversationFirst, treeShell(t, "t3", "cat reply.md && demi todo list"), treeSay("checked"))
	_, err = socket.Chat(ctx, "m3", "Check")
	wireMust(t, err)
	roots := scripts.requests(conversationFirst, true)
	checked := treeRequestText(roots[len(roots)-1])
	if !strings.Contains(checked, "from the child") || !strings.Contains(checked, "root-only") || strings.Contains(checked, "child-only") {
		t.Fatalf("parent files/todos: %s", checked)
	}
	history := conversationTranscript(ctx, t, b, &s, conversationFirst)
	conversationEqual(t, len(history.Subagents), 1)
	conversationEqual(t, history.Subagents[0].Subagent.Description, "reader")
	conversationEqual(t, history.Subagents[0].Subagent.Phase, framewire.JobPhaseCompleted)
}

func TestForkLeavesRunningChildWithSource(t *testing.T) {
	ctx, b, s, scripts, root := conversationTree(t)
	socket := conversationOpen(ctx, t, b, &s, conversationFirst)
	scripts.child(treeShell(t, "c1", `until [ -f go ]; do sleep 0.05; done`), treeSay("the child's result"))
	scripts.root(conversationFirst, treeShell(t, "t1", `demi agent spawn --description worker <<< 'Wait for the file'`), treeSay("the child is still working"), treeSay("received"))
	_, err := socket.Chat(ctx, "m1", "Start a worker")
	wireMust(t, err)
	live, err := socket.Live(ctx)
	wireMust(t, err)
	texts := conversationTexts(live)
	conversationFork(ctx, t, b, &s, conversationSecond, texts[len(texts)-1], 201)
	before := conversationTranscript(ctx, t, b, &s, conversationSecond)
	found := false
	for _, block := range before.Blocks {
		if _, ok := block.(*core.ToolCallBlock); ok {
			found = true
		}
	}
	if !found {
		t.Fatal("fork lost spawn call")
	}
	conversationEqual(t, len(before.Subagents), 0)
	wireMust(t, os.WriteFile(filepath.Join(root, "go"), nil, 0644))
	_, err = socket.UntilIdle(ctx)
	wireMust(t, err)
	conversationEqual(t, len(conversationTranscript(ctx, t, b, &s, conversationFirst).Subagents), 1)
	conversationEqual(t, conversationTranscript(ctx, t, b, &s, conversationSecond), before)
	fork := conversationOpen(ctx, t, b, &s, conversationSecond)
	scripts.root(conversationSecond, treeShell(t, "f1", "demi agent list"), treeSay("an empty tree"))
	_, err = fork.Chat(ctx, "m2", "Who works for you?")
	wireMust(t, err)
	requests := scripts.requests(conversationSecond, true)
	var listed string
	for _, item := range requests[len(requests)-1].Items {
		if result, ok := item.(*provider.ToolResult); ok && result.ToolUseID == "f1" {
			for _, part := range result.Output {
				if text, ok := part.(*provider.TextPart); ok {
					listed += text.Text
				}
			}
		}
	}
	if !strings.Contains(listed, "(root session)") || strings.Contains(listed, "worker") {
		t.Fatalf("fork's tree: %s", listed)
	}
}
