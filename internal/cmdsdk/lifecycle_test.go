package cmdsdk

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"net"
	"net/http"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/commandwire"
)

func TestConversationEndpointHasNoGrantsAndValidatesReleaseIdentity(t *testing.T) {
	c, done := connected(t, fixture{})
	for _, q := range []commandwire.ConversationRequest{&commandwire.ConversationQuery{}, &commandwire.ConversationRelease{Conversation: "unknown"}, &commandwire.ConversationRelease{Conversation: "unknown"}} {
		_, o, err := c.Conversation(t.Context(), q)
		must(t, err)
		want := "{}"
		if _, ok := q.(*commandwire.ConversationQuery); ok {
			want = `{"conversations":[]}`
		}
		if got := string(completed(t, o)); got != want {
			t.Fatalf("got %s, want %s", got, want)
		}
	}
	for _, test := range []struct {
		path, body string
		status     int
	}{
		{"/v1/resource", `{"operation":"acquire"}`, 404},
		{commandwire.ConversationPath, `{"operation":"release"}`, 400},
		{commandwire.ConversationPath, `{"operation":"acquire","conversation":"one"}`, 400},
		{commandwire.ConversationPath, `{"operation":"release","conversation":""}`, 400},
		{commandwire.ConversationPath, `{"operation":"status","resource":"one"}`, 400},
	} {
		b := make([]byte, 4)
		binary.BigEndian.PutUint32(b, uint32(len(test.body)))
		b = append(b, []byte(test.body)...)
		_, _, err := c.invokeAt(t.Context(), test.path, b, true)
		var rejected *RejectedError
		if !errors.As(err, &rejected) || rejected.Status != test.status {
			t.Fatalf("%s: %v", test.body, err)
		}
	}
	must(t, c.Shutdown(t.Context()))
	must(t, <-done)
}

type lifecycle struct {
	fixture
	started, cancelled, closed chan struct{}
}

func (h *lifecycle) Conversation(ctx context.Context, c ConversationContext) (commandwire.Completion, error) {
	if q, ok := c.Request.(*commandwire.ConversationRelease); ok && q.Conversation == "wait" {
		close(h.started)
		<-ctx.Done()
		close(h.cancelled)
		return commandwire.Completion{}, ctx.Err()
	}
	return commandwire.Completion{}, errors.New("profile cleanup failed")
}
func (h *lifecycle) Close(context.Context) error {
	close(h.closed)
	return nil
}
func TestConversationCancellationJoinsHookAndCleanupFailureRetiresService(t *testing.T) {
	h := &lifecycle{started: make(chan struct{}), cancelled: make(chan struct{}), closed: make(chan struct{})}
	c, done := connected(t, h)
	i, _, err := c.Conversation(t.Context(), &commandwire.ConversationRelease{Conversation: "wait"})
	must(t, err)
	<-h.started
	i.Cancel()
	<-h.cancelled
	_, err = c.Info(t.Context())
	must(t, err)
	_, o, err := c.Conversation(t.Context(), &commandwire.ConversationRelease{Conversation: "fail"})
	if err == nil {
		r, _ := o.Next(t.Context())
		if final, ok := r.(commandwire.Completed); ok && final.Completion.ExitCode == 0 {
			t.Fatal("cleanup reported success")
		}
	}
	err = <-done
	var cleanup *ConversationCleanupError
	if !errors.As(err, &cleanup) || !strings.Contains(err.Error(), "profile cleanup failed") {
		t.Fatalf("cleanup: %v", err)
	}
	<-h.closed
}

type drawing struct {
	numbers *Numbers
	drawing chan struct{}
}

func (h *drawing) SetNumbers(n *Numbers) { h.numbers = n }
func (h *drawing) Operations() []string  { return []string{"draw"} }
func (h *drawing) Invoke(ctx context.Context, c InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	select {
	case h.drawing <- struct{}{}:
	default:
	}
	first, err := h.numbers.Draw(ctx, c.Request.Context.Conversation, commandwire.TabSequence, 4)
	if err != nil {
		return commandwire.Completion{}, err
	}
	return commandwire.Completion{}, c.Output.Stdout(ctx, []byte(fmt.Sprint(first)))
}
func TestDrawsWaitForNumbersStreamContinuePerConversationAndShutdownEndsIt(t *testing.T) {
	h := &drawing{drawing: make(chan struct{}, 1)}
	c, done := connected(t, h)
	_, early, err := c.Invoke(t.Context(), invocation("draw"))
	must(t, err)
	<-h.drawing
	stream, err := c.Numbers(t.Context())
	must(t, err)
	answering := make(chan error, 1)
	next := map[string]uint64{}
	// The callback serializes its shared sequence state, independently of the stream's workers.
	var lock = make(chan struct{}, 1)
	go func() {
		answering <- stream.AnswerNumbers(t.Context(), func(_ context.Context, q commandwire.NumbersRequest) (uint64, error) {
			lock <- struct{}{}
			defer func() { <-lock }()
			first := next[q.Conversation]
			if first == 0 {
				first = 1
			}
			next[q.Conversation] = first + uint64(q.Count)
			return first, nil
		})
	}()
	if got := string(completed(t, early)); got != "1" {
		t.Fatal(got)
	}
	for _, test := range []struct{ conversation, want string }{{"one", "5"}, {"two", "1"}} {
		m := invocation("draw")
		m.Context.Conversation = test.conversation
		_, o, err := c.Invoke(t.Context(), m)
		must(t, err)
		if got := string(completed(t, o)); got != test.want {
			t.Fatalf("got %s want %s", got, test.want)
		}
	}
	_, err = c.Numbers(t.Context())
	var rejected *RejectedError
	if !errors.As(err, &rejected) || rejected.Status != http.StatusConflict {
		t.Fatalf("duplicate stream: %v", err)
	}
	must(t, c.Shutdown(t.Context()))
	must(t, <-answering)
	must(t, <-done)
}

type artifactHandler struct{ artifacts *Artifacts }

func (h *artifactHandler) SetArtifacts(a *Artifacts) { h.artifacts = a }
func (h *artifactHandler) Operations() []string      { return []string{"install", "installed"} }
func (h *artifactHandler) Invoke(ctx context.Context, c InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	var value string
	var err error
	if c.Request.Operation == "install" {
		value, err = h.artifacts.Install(ctx, commandwire.ArtifactInstall{Invocation: c.Request.InvocationID, Name: "tool", Version: "1", SHA256: strings.Repeat("a", 64), Size: 1, Form: &commandwire.ArtifactFile{}})
	} else {
		var installed []commandwire.InstalledArtifact
		installed, err = h.artifacts.Installed(ctx, "tool")
		if err == nil && len(installed) > 0 {
			value = installed[0].Path
		}
	}
	if err != nil {
		return commandwire.Completion{}, err
	}
	return commandwire.Completion{}, c.Output.Stdout(ctx, []byte(value))
}
func TestArtifactsUseRunnerAnswersAndEndAtShutdown(t *testing.T) {
	h := &artifactHandler{}
	c, done := connected(t, h)
	_, early, err := c.Invoke(t.Context(), invocation("install"))
	must(t, err)
	stream, err := c.Artifacts(t.Context())
	must(t, err)
	answered := make(chan error, 1)
	go func() {
		answered <- stream.AnswerArtifacts(t.Context(), func(_ context.Context, q commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error) {
			path := "/runner/installed/tool"
			if q.Install != nil {
				if q.Install.Invocation != "install" {
					return commandwire.ArtifactAnswer{}, errors.New("wrong invocation")
				}
				return commandwire.ArtifactAnswer{Path: &path}, nil
			}
			values := []commandwire.InstalledArtifact{{Version: "1", SHA256: strings.Repeat("a", 64), Path: path}}
			return commandwire.ArtifactAnswer{Installed: &values}, nil
		})
	}()
	if got := string(completed(t, early)); got != "/runner/installed/tool" {
		t.Fatal(got)
	}
	_, o, err := c.Invoke(t.Context(), invocation("installed"))
	must(t, err)
	if got := string(completed(t, o)); got != "/runner/installed/tool" {
		t.Fatal(got)
	}
	_, err = c.Artifacts(t.Context())
	var rejected *RejectedError
	if !errors.As(err, &rejected) || rejected.Status != 409 {
		t.Fatal(err)
	}
	must(t, c.Shutdown(t.Context()))
	must(t, <-answered)
	must(t, <-done)
	if _, err = h.artifacts.Installed(t.Context(), "tool"); err == nil {
		t.Fatal("source survived shutdown")
	}
}
func TestSideStreamRefusesBeyond32InFlight(t *testing.T) {
	h := &drawing{drawing: make(chan struct{}, 1)}
	c, done := connected(t, h)
	_, err := c.Info(t.Context())
	must(t, err)
	stream, err := c.Numbers(t.Context())
	must(t, err)
	started := make(chan struct{}, 32)
	release := make(chan struct{})
	answered := make(chan error, 1)
	go func() {
		answered <- stream.AnswerNumbers(t.Context(), func(ctx context.Context, _ commandwire.NumbersRequest) (uint64, error) {
			started <- struct{}{}
			select {
			case <-release:
				return 1, nil
			case <-ctx.Done():
				return 0, ctx.Err()
			}
		})
	}()
	results := make(chan error, 32)
	for range 32 {
		go func() {
			_, err := h.numbers.Draw(t.Context(), "one", commandwire.TabSequence, 1)
			results <- err
		}()
	}
	for range 32 {
		<-started
	}
	_, err = h.numbers.Draw(t.Context(), "one", commandwire.TabSequence, 1)
	if err == nil || !strings.Contains(err.Error(), "too many numbers requests") {
		t.Fatalf("excess request: %v", err)
	}
	close(release)
	for range 32 {
		must(t, <-results)
	}
	must(t, c.Shutdown(t.Context()))
	must(t, <-answered)
	must(t, <-done)
}

type stubborn struct {
	fixture
	started, release, finished chan struct{}
}

func (h *stubborn) Invoke(context.Context, InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	close(h.started)
	<-h.release
	close(h.finished)
	return commandwire.Completion{}, nil
}
func TestCancellationDeadlineRequiresServiceRetirement(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		h := &stubborn{started: make(chan struct{}), release: make(chan struct{}), finished: make(chan struct{})}
		c, done := connected(t, h)
		i, _, err := c.Invoke(t.Context(), invocation("hold"))
		must(t, err)
		<-h.started
		start := time.Now()
		i.Cancel()
		err = <-done
		// Go cannot abort arbitrary handler work. The executable owner must exit;
		// the controlled fixture releases it here so the test can audit cleanup.
		close(h.release)
		<-h.finished
		if !errors.Is(err, ErrCancellationDeadline) {
			t.Fatalf("retirement: %v", err)
		}
		if time.Since(start) != cancelTimeout {
			t.Fatalf("grace %s", time.Since(start))
		}
	})
}
func TestConnectUsesCallerDeadline(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		left, right := net.Pipe()
		defer func() { _ = right.Close() }()
		ctx, cancel := context.WithTimeout(t.Context(), 37*time.Second)
		defer cancel()
		start := time.Now()
		_, err := Connect(ctx, left)
		if err == nil {
			t.Fatal("connected to a peer that read nothing")
		}
		if time.Since(start) != 37*time.Second {
			t.Fatalf("caller handshake deadline: elapsed %s", time.Since(start))
		}
	})
}

// Both the response headers and body may wait longer than the former SDK
// timeout. Only the caller decides when to give up waiting for service info.
func TestInfoUsesCallerDeadline(t *testing.T) {
	for _, headers := range []bool{false, true} {
		t.Run(fmt.Sprint("headers=", headers), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				left, right := net.Pipe()
				listener := &oneListener{conn: right, closed: make(chan struct{})}
				handled := make(chan struct{})
				server := &http.Server{Protocols: protocols(), HTTP2: h2Config(false), Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					defer close(handled)
					if headers {
						w.WriteHeader(http.StatusOK)
						if err := http.NewResponseController(w).Flush(); err != nil {
							return
						}
					}
					<-r.Context().Done()
				})}
				served := make(chan error, 1)
				go func() { served <- server.Serve(listener) }()
				defer func() {
					must(t, server.Close())
					if err := <-served; !errors.Is(err, http.ErrServerClosed) {
						t.Error(err)
					}
				}()
				client, err := Connect(t.Context(), left)
				must(t, err)
				defer func() { _ = client.Close() }()
				ctx, cancel := context.WithTimeout(t.Context(), 37*time.Second)
				defer cancel()
				start := time.Now()
				_, err = client.Info(ctx)
				if !errors.Is(err, context.DeadlineExceeded) {
					t.Fatalf("info error: %v", err)
				}
				if time.Since(start) != 37*time.Second {
					t.Fatalf("caller info deadline: elapsed %s", time.Since(start))
				}
				<-handled
			})
		})
	}
}
