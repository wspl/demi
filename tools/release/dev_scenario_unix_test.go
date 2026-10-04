//go:build !windows

package main

import (
	"bytes"
	"context"
	"errors"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// devOutput captures concurrent child output and signals the printed readiness
// event without polling or holding a lock across a blocking write.
type devOutput struct {
	mu     sync.Mutex
	buffer bytes.Buffer
	ready  chan string
	once   sync.Once
}

func (o *devOutput) Write(p []byte) (int, error) {
	o.mu.Lock()
	defer o.mu.Unlock()
	n, err := o.buffer.Write(p)
	if strings.Contains(o.buffer.String(), "Ctrl-C stops the backend.") {
		o.once.Do(func() { o.ready <- o.buffer.String() })
	}
	return n, err
}

func (o *devOutput) text() string {
	o.mu.Lock()
	defer o.mu.Unlock()
	return o.buffer.String()
}

// Cost: dev builds the backend, manager, runner and command programs once,
// then uses Go's shared build cache. The scenario runs dev in process, uses a
// local Chrome archive and Echo, and waits on output and WebSocket events.
func TestDevSeededAccountEchoAndShutdown(t *testing.T) {
	root, err := repository()
	if err != nil {
		t.Fatal(err)
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := listener.Addr().(*net.TCPAddr).Port
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
	archive := []byte("offline Chrome resource: this scenario never opens a browser")
	archivePath := filepath.Join(t.TempDir(), "chrome.zip")
	writeFixture(t, archivePath, archive)
	digest, err := measureExecutable(t.Context(), archivePath)
	if err != nil {
		t.Fatal(err)
	}
	cache := filepath.Join(root, ".cache/resources", digest.SHA256)
	writeFixture(t, cache, archive)
	t.Cleanup(func() {
		if err := os.Remove(cache); err != nil {
			t.Error(err)
		}
	})
	target, err := commandproto.HostTarget()
	if err != nil {
		t.Fatal(err)
	}
	output := &devOutput{ready: make(chan string, 1)}
	app := application{
		Root: root,
		Out:  output,
		Err:  output,
		chromeRelease: func() (browserproto.BrowserRelease, error) {
			return browserproto.BrowserRelease{
				Version: "153.0.8010.36",
				Platforms: []browserproto.ReleasePlatform{
					{
						Target:     string(target),
						URL:        "https://unused.invalid/chrome.zip",
						Size:       digest.Size,
						SHA256:     digest.SHA256,
						Executable: "chrome",
					},
				},
			}, nil
		},
	}
	// The development entry is only seeded: no turn uses it, so its endpoint
	// is never called.
	developmentURL, err := webapiproto.ParseEndpointURL("https://models.example.test/v1")
	if err != nil {
		t.Fatal(err)
	}
	development := devProvider{
		BaseURL:       developmentURL,
		APIKey:        "sk-development",
		Model:         "vendor/model-flash",
		ContextWindow: 1000000,
	}
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	done := make(chan struct{})
	var devErr error
	go func() {
		defer close(done)
		devErr = app.dev(ctx, devOptions{Port: uint(port), Provider: &development})
	}()
	defer func() {
		cancel()
		<-done
		if devErr != nil {
			t.Errorf("dev: %v\n%s", devErr, output.text())
		}
	}()
	var printed string
	select {
	case printed = <-output.ready:
	case <-done:
		t.Fatalf("dev exited before readiness: %v\n%s", devErr, output.text())
	}
	value := func(prefix string) string {
		for line := range strings.SplitSeq(printed, "\n") {
			if v, ok := strings.CutPrefix(line, prefix); ok {
				return v
			}
		}
		t.Fatalf("missing %q in %s", prefix, printed)
		return ""
	}
	origin := value("The development backend serves at ")
	data := value("  Data:    ")
	account := strings.SplitN(value("  Account: "), ", password ", 2)
	provider := strings.TrimSuffix(strings.Fields(value("  Model:   echo of the provider entry "))[0], ",")
	client, err := devClient()
	if err != nil {
		t.Fatal(err)
	}
	defer client.CloseIdleConnections()
	b := &backendtest.TestBackend{URL: origin, HTTP: client}
	session, err := b.Login(ctx, account[0], account[1])
	if err != nil {
		t.Fatal(err)
	}
	developmentID := strings.Fields(value("  Model:   vendor/model-flash of the provider entry "))[0]
	answer, err := b.Read(ctx, "/api/providers", &session)
	if err != nil {
		t.Fatal(err)
	}
	entries, err := webapiproto.DecodeProviders(answer.Body)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries.Providers) != 2 {
		t.Fatalf("provider entries: %s", answer.Body)
	}
	seeded := entries.Providers[1]
	wire := types.WireAPIChatCompletions
	if string(seeded.ID) != developmentID || seeded.ProviderType != "openai" || seeded.Label != "Development" ||
		seeded.WireAPI == nil || *seeded.WireAPI != wire || seeded.BaseURL == nil ||
		*seeded.BaseURL != developmentURL || seeded.Models == nil || len(*seeded.Models) != 1 ||
		(*seeded.Models)[0].ID != "vendor/model-flash" || (*seeded.Models)[0].ContextWindow != 1000000 {
		t.Fatalf("development entry: %s", answer.Body)
	}
	if strings.Contains(printed, "sk-development") {
		t.Fatal("the summary printed the development key")
	}
	id := webapiproto.ConversationID("b1a62b67-0182-4d89-8319-fd5be3a24894")
	body, err := contract.EncodeJSON(webapiproto.CreateConversation{ID: id})
	if err != nil {
		t.Fatal(err)
	}
	answer, err = b.Post(ctx, "/api/conversations", &session, body)
	if err != nil {
		t.Fatal(err)
	}
	if answer.Status != http.StatusCreated {
		t.Fatalf("create: %d %s", answer.Status, answer.Body)
	}
	if _, err := webapiproto.DecodeConversationAnswer(answer.Body); err != nil {
		t.Fatal(err)
	}
	body, err = contract.EncodeJSON(
		webapiproto.ConversationPatch{
			Model: &webapiproto.ModelChoice{ProviderID: webapiproto.ProviderID(provider), ModelID: "echo"},
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	path := "/api/conversations/" + string(id)
	answer, err = b.Patch(ctx, path, &session, body)
	if err != nil {
		t.Fatal(err)
	}
	if answer.Status != http.StatusOK {
		t.Fatalf("select Echo: %d %s", answer.Status, answer.Body)
	}
	if _, err := webapiproto.DecodeConversationUpdate(answer.Body); err != nil {
		t.Fatal(err)
	}
	socket, response, err := websocket.Dial(
		ctx,
		b.WSURL(path+"/stream"),
		&websocket.DialOptions{
			HTTPClient: client,
			HTTPHeader: http.Header{"Origin": {origin}, "Cookie": {session.Cookie}},
		},
	)
	if err != nil {
		if response != nil {
			_ = response.Body.Close()
		}
		t.Fatal(err)
	}
	defer func() { _ = socket.CloseNow() }()
	send := func(frame conversationproto.ClientFrame) {
		t.Helper()
		data, err := contract.EncodeJSON(frame)
		if err != nil {
			t.Fatal(err)
		}
		if err := socket.Write(ctx, websocket.MessageText, data); err != nil {
			t.Fatal(err)
		}
	}
	next := func() conversationproto.ServerFrame {
		t.Helper()
		_, data, err := socket.Read(ctx)
		if err != nil {
			t.Fatalf("conversation: %v\n%s", err, output.text())
		}
		frame, err := conversationproto.DecodeServerFrame(data)
		if err != nil {
			t.Fatal(err)
		}
		if refusal, ok := frame.(*conversationproto.RejectedFrame); ok {
			t.Fatalf("rejected: %+v", refusal)
		}
		if failure, ok := frame.(*conversationproto.ErrorFrame); ok {
			t.Fatalf("backend error: %+v", failure)
		}
		return frame
	}
	send(&conversationproto.OpenFrame{})
	for {
		if phase, ok := next().(*conversationproto.PhaseFrame); ok && phase.Phase == types.SessionPhaseIdle {
			break
		}
	}
	send(
		&conversationproto.SendFrame{
			MessageID: "dev-hello",
			Content:   []conversationproto.ClientContent{&conversationproto.TextContent{Text: "hello"}},
		},
	)
	running := false
	for {
		if phase, ok := next().(*conversationproto.PhaseFrame); ok {
			if phase.Phase == types.SessionPhaseRunning {
				running = true
			}
			if running && phase.Phase == types.SessionPhaseIdle {
				break
			}
		}
	}
	answer, err = b.Read(ctx, path+"/transcript", &session)
	if err != nil {
		t.Fatal(err)
	}
	transcript, err := webapiproto.DecodeTranscript(answer.Body)
	if err != nil {
		t.Fatalf("transcript: %v: %s", err, answer.Body)
	}
	var reply strings.Builder
	for _, block := range transcript.Blocks {
		if text, ok := block.(*types.TextBlock); ok {
			reply.WriteString(text.Text)
		}
	}
	if reply.String() != "Echo: hello" {
		t.Fatalf("reply = %q", reply.String())
	}
	// Snapshot the actual process tree, including any separately grouped
	// runners, before cancellation; then require each PID to be gone.
	pids := devProcessIDs(t)
	if len(pids) < 2 {
		t.Fatalf("expected backend and manager: %v", pids)
	}
	_ = socket.CloseNow()
	cancel()
	<-done
	for _, pid := range pids {
		if err := syscall.Kill(pid, 0); !errors.Is(err, syscall.ESRCH) {
			t.Errorf("dev child %d remains: %v", pid, err)
		}
	}
	if _, err := os.Stat(data); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("data directory remains: %s (%v)", data, err)
	}
}

// devProcessIDs records the development programs and their descendants so a
// leaked runner or command process is visible after shutdown.
func devProcessIDs(t *testing.T) []int {
	t.Helper()
	cmd := exec.CommandContext(t.Context(), "ps", "-axo", "pid=,ppid=,command=")
	data, err := cmd.Output()
	if err != nil {
		t.Fatal(err)
	}
	parents := make(map[int]int)
	selected := map[int]bool{os.Getpid(): true}
	for line := range strings.SplitSeq(string(data), "\n") {
		fields := strings.Fields(line)
		if len(fields) < 3 {
			continue
		}
		pid, err := strconv.Atoi(fields[0])
		if err != nil {
			t.Fatal(err)
		}
		parent, err := strconv.Atoi(fields[1])
		if err != nil {
			t.Fatal(err)
		}
		parents[pid] = parent
	}
	for changed := true; changed; {
		changed = false
		for pid, parent := range parents {
			if selected[parent] && !selected[pid] {
				selected[pid] = true
				changed = true
			}
		}
	}
	delete(selected, os.Getpid())
	delete(selected, cmd.Process.Pid)
	var result []int
	for pid := range selected {
		result = append(result, pid)
	}
	return result
}
