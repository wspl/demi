package backendtest_test

import (
	"encoding/json/v2"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// tree is a scripted vendor that answers each node of a conversation's tree from
// a script of its own: a conversation's root by the conversation, and a child by
// the task it was given, its first message. Requests of one tree interleave in
// no order a scenario could script, so the vendor tells the nodes apart by what
// they carry.
type tree struct {
	mu       sync.Mutex
	roots    map[string][]*scripted.Response
	children map[string][]*scripted.Response
	// asked is what each request carried, by the node that sent it.
	asked map[string][]string
}

func newTree() *tree {
	return &tree{
		roots:    map[string][]*scripted.Response{},
		children: map[string][]*scripted.Response{},
		asked:    map[string][]string{},
	}
}

// root queues the answers of the conversation's root.
func (tr *tree) root(conversation string, answers ...*scripted.Response) {
	tr.mu.Lock()
	defer tr.mu.Unlock()
	tr.roots[conversation] = append(tr.roots[conversation], answers...)
}

// child queues the answers of the child whose task is task.
func (tr *tree) child(task string, answers ...*scripted.Response) {
	tr.mu.Lock()
	defer tr.mu.Unlock()
	tr.children[task] = append(tr.children[task], answers...)
}

// messageText is the content of the message of a Messages API request that is
// first, or last.
func messageText(body any, last bool) string {
	messages, _ := backendtest.At(body, "messages").([]any)
	if len(messages) == 0 {
		return ""
	}
	message := messages[0]
	if last {
		message = messages[len(messages)-1]
	}
	return string(backendtest.Marshal(backendtest.At(message, "content")))
}

// handle answers a request from the script of the node that sent it.
func (tr *tree) handle(request scripted.Request) *scripted.Response {
	var body any
	if err := json.Unmarshal(request.Body, &body); err != nil {
		return nil
	}
	tr.mu.Lock()
	defer tr.mu.Unlock()
	node := ""
	for task := range tr.children {
		if strings.Contains(messageText(body, false), task) {
			node = "child:" + task
			queue := tr.children[task]
			tr.asked[node] = append(tr.asked[node], string(request.Body))
			if len(queue) == 0 {
				return nil
			}
			tr.children[task] = queue[1:]
			return queue[0]
		}
	}
	// A conversation's root is named by a message only it holds.
	node = "first"
	if strings.Contains(string(request.Body), "Who works for you?") {
		node = "second"
	}
	tr.asked[node] = append(tr.asked[node], string(request.Body))
	queue := tr.roots[node]
	if len(queue) == 0 {
		return nil
	}
	tr.roots[node] = queue[1:]
	return queue[0]
}

// askedBy returns what the requests of the node carried, in order.
func (tr *tree) askedBy(node string) []string {
	tr.mu.Lock()
	defer tr.mu.Unlock()
	return append([]string(nil), tr.asked[node]...)
}

// treeBackend starts a backend whose master has the conversation convFirst on
// the scripted vendor's model and a paired device's home as its directory.
func treeBackend(t *testing.T, tr *tree) (*backendtest.Backend, *backendtest.Session, string) {
	t.Helper()
	vendor := scripted.StartVendor(t)
	vendor.Handle(tr.handle)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	paired := onDeviceConversation(t, b, master, convFirst)
	return b, master, paired.Home()
}

// waitForGo is the shell that waits until the file go appears where it works.
const waitForGo = "until [ -f go ]; do sleep 0.05; done"

func shellAnswer(id, script string) *scripted.Response {
	return backendtest.ShellCall(id, script, 60*time.Second)
}

// Cost: one backend, a scripted vendor and a real runner, several seconds: a
// parent and its child run five shell jobs, the child's across its parent's
// turns.
func TestAChildWorksInItsParentsFilesKeepsItsOwnTodosAndRunsOnAfterItsSpawn(t *testing.T) {
	t.Parallel()
	tr := newTree()
	b, master, root := treeBackend(t, tr)
	socket := b.Connect(master, convFirst)
	socket.Open()

	tr.root("first",
		shellAnswer("t1", `printf 'the answer is 42\n' > notes.md && demi todo add root-only`),
		backendtest.Say("written"),
	)
	socket.Chat("m1", "Write the notes")

	// The child waits until the test lets it go, long after the spawn command
	// exited; then it reads the parent's file, writes one of its own, and adds a
	// todo through a shell on the conversation's device.
	child := waitForGo + `; cat notes.md && printf 'from the child\n' > reply.md && ` +
		`demi host shell --host laptop "demi todo add child-only" && demi todo list`
	const task = "Read notes.md and answer"
	tr.child(task, shellAnswer("c1", child), backendtest.Say("the file says 42"))
	tr.root("first",
		shellAnswer("t2", "demi agent spawn --description reader <<< '"+task+"'"),
		backendtest.Say("dispatched"),
		// The child's completion wakes the idle parent.
		backendtest.Say("received"),
	)
	socket.Chat("m2", "Delegate the reading")
	if err := os.WriteFile(filepath.Join(root, "go"), nil, 0o644); err != nil {
		t.Fatal(err)
	}
	socket.UntilIdle()

	children := tr.askedBy("child:" + task)
	read := scenarioItem(t, children, len(children)-1)
	contains(t, read, "the answer is 42", "child-only")
	if strings.Contains(read, "root-only") {
		t.Fatalf("the child's todos are its own: %s", read)
	}
	if got := readFile(t, filepath.Join(root, "reply.md")); got != "from the child\n" {
		t.Fatalf("reply.md holds %q", got)
	}

	tr.root("first", shellAnswer("t3", "cat reply.md && demi todo list"), backendtest.Say("checked"))
	socket.Chat("m3", "Check")
	asked := tr.askedBy("first")
	checked := scenarioItem(t, asked, len(asked)-1)
	contains(t, checked, "from the child", "root-only")
	if strings.Contains(checked, "child-only") {
		t.Fatalf("the parent's todos are its own: %s", checked)
	}

	history := b.Get("/api/conversations/"+convFirst+"/transcript", master).Expect(http.StatusOK)
	jobs, _ := history.At("subagents").([]any)
	if len(jobs) != 1 || backendtest.At(jobs[0], "subagent.description") != "reader" || backendtest.At(jobs[0], "subagent.phase") != "completed" {
		t.Fatalf("the subagents are %v", jobs)
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, about two seconds: the
// parent's and the child's shell jobs run on a real device, the child's until
// the Fork is taken.
func TestAForkTakenWhileAChildRunsLeavesTheChildWithItsSource(t *testing.T) {
	t.Parallel()
	tr := newTree()
	b, master, root := treeBackend(t, tr)
	socket := b.Connect(master, convFirst)
	socket.Open()
	const task = "Wait for the file"
	tr.child(task, shellAnswer("c1", waitForGo), backendtest.Say("the child's result"))
	tr.root("first",
		shellAnswer("t1", "demi agent spawn --description worker <<< '"+task+"'"),
		backendtest.Say("the child is still working"),
		backendtest.Say("received"),
	)
	socket.Chat("m1", "Start a worker")

	blocks := socket.Live()
	var text string
	for index := len(blocks) - 1; index >= 0 && text == ""; index-- {
		if backendtest.At(blocks[index], "type") == "text" {
			text = backendtest.At(blocks[index], "id").(string)
		}
	}
	if text == "" {
		t.Fatal("the parent answered no text")
	}
	b.Post("/api/conversations/"+convFirst+"/fork", master, backendtest.Map{"id": convSecond, "blockId": text}).Expect(http.StatusCreated)
	// The Fork keeps the call that spawned the child, and no child.
	before := b.Get("/api/conversations/"+convSecond+"/transcript", master).Expect(http.StatusOK)
	if !containsKind(backendtest.BlockKinds(before.At("blocks").([]any)), "tool_call") {
		t.Fatalf("the fork's history is %v", backendtest.BlockKinds(before.At("blocks").([]any)))
	}
	if subagents, _ := before.At("subagents").([]any); len(subagents) != 0 {
		t.Fatalf("the fork has subagents: %v", subagents)
	}

	// The child finishes into its source alone.
	if err := os.WriteFile(filepath.Join(root, "go"), nil, 0o644); err != nil {
		t.Fatal(err)
	}
	socket.UntilIdle()
	source := b.Get("/api/conversations/"+convFirst+"/transcript", master).Expect(http.StatusOK)
	if subagents, _ := source.At("subagents").([]any); len(subagents) != 1 {
		t.Fatalf("the source has subagents: %v", subagents)
	}
	backendtest.AssertJSON(t, b.Get("/api/conversations/"+convSecond+"/transcript", master).Value(), before.Value())

	// The Fork's tree has no child to list.
	fork := b.Connect(master, convSecond)
	fork.Open()
	tr.root("second", shellAnswer("f1", "demi agent list"), backendtest.Say("an empty tree"))
	fork.Chat("m2", "Who works for you?")
	asked := tr.askedBy("second")
	last := scenarioItem(t, asked, len(asked)-1)
	var request any
	if err := json.Unmarshal([]byte(last), &request); err != nil {
		t.Fatal(err)
	}
	listed := scripted.ToolResult(t, request, "f1")
	if !strings.Contains(listed, "(root session)") || strings.Contains(listed, "worker") {
		t.Fatalf("the fork lists %s", listed)
	}
	b.Stop()
}

func containsKind(kinds []string, kind string) bool {
	for _, got := range kinds {
		if got == kind {
			return true
		}
	}
	return false
}
