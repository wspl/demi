package cmdsdk

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/commandwire"
)

func TestConversationEndpointHasNoGrantsAndValidatesReleaseIdentity(t *testing.T) {
	c, done := connected(t, fixture{})
	for _, q := range []commandwire.ConversationRequest{
		&commandwire.ConversationQuery{},
		&commandwire.ConversationRelease{
			Conversation: "unknown",
		},
		&commandwire.ConversationRelease{
			Conversation: "unknown",
		},
	} {
		_, o, err := c.Conversation(t.Context(), q)
		must(t, err)
		want := "{}"
		if _, ok := q.(*commandwire.ConversationQuery); ok {
			want = `{"conversations":[]}`
		}
		for _, expected := range []commandwire.Record{
			commandwire.Stdout(want),
			commandwire.Completed{
				Completion: commandwire.Completion{},
			},
		} {
			got, err := o.Next(t.Context())
			must(t, err)
			if !reflect.DeepEqual(got, expected) {
				t.Fatalf("record %#v, want %#v", got, expected)
			}
		}
		if _, err := o.Next(t.Context()); !errors.Is(err, io.EOF) {
			t.Fatalf("conversation end: %v", err)
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
	info, err := c.Info(t.Context())
	must(t, err)
	if !reflect.DeepEqual(info.Operations, h.Operations()) {
		t.Fatalf("operations: %v", info.Operations)
	}
	_, o, err := c.Conversation(t.Context(), &commandwire.ConversationRelease{Conversation: "fail"})
	must(t, err)
	r, _ := o.Next(t.Context())
	if final, ok := r.(commandwire.Completed); ok && final.Completion.ExitCode != 1 {
		t.Fatal("cleanup reported success")
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

func (h *drawing) Invoke(
	ctx context.Context,
	c InvocationContext[commandwire.Invocation],
) (commandwire.Completion, error) {
	select {
	case h.drawing <- struct{}{}:
	default:
	}
	count := uint32(4)
	if string(c.Request.Args) == `{"count":1}` {
		count = 1
	}
	first, err := h.numbers.Draw(ctx, c.Request.Context.Conversation, commandwire.TabSequence, count)
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
	lock := make(chan struct{}, 1)
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
		m.Args = []byte(`{"count":1}`)
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

func (h *artifactHandler) Invoke(
	ctx context.Context,
	c InvocationContext[commandwire.Invocation],
) (commandwire.Completion, error) {
	var value string
	var err error
	if c.Request.Operation == "install" {
		value, err = h.artifacts.Install(
			ctx,
			commandwire.ArtifactInstall{
				Invocation: c.Request.InvocationID,
				Name:       "tool",
				Version:    "1",
				SHA256:     strings.Repeat("a", 64),
				Size:       1,
				Form:       &commandwire.ArtifactFile{},
			},
		)
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
		answered <- stream.AnswerArtifacts(t.Context(), func(
			_ context.Context,
			q commandwire.ArtifactRequest,
		) (commandwire.ArtifactAnswer, error) {
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
		answered <- stream.AnswerNumbers(t.Context(), func(
			ctx context.Context,
			_ commandwire.NumbersRequest,
		) (uint64, error) {
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
				server := &http.Server{
					Protocols: protocols(),
					HTTP2:     h2Config(false),
					Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
						defer close(handled)
						if headers {
							w.WriteHeader(http.StatusOK)
							if err := http.NewResponseController(w).Flush(); err != nil {
								return
							}
						}
						<-r.Context().Done()
					}),
				}
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

// A custom peer can repeat IDs that the SDK's own Numbers source never repeats.
// Exercise AnswerNumbers through records, including an answer queued for a pull.
type numberPeer struct {
	duplicate <-chan struct{}
	reuse     <-chan struct{}
	replies   chan<- commandwire.NumbersAnswer
}

func (*numberPeer) Operations() []string { return []string{"numbers-peer"} }

func (p *numberPeer) Invoke(
	ctx context.Context,
	c InvocationContext[commandwire.Invocation],
) (commandwire.Completion, error) {
	request := []byte(`{"id":7,"conversation":"one","sequence":"tab","count":1}`)
	if err := c.Output.Stdout(ctx, request); err != nil {
		return commandwire.Completion{}, err
	}
	select {
	case <-p.duplicate:
	case <-ctx.Done():
		return commandwire.Completion{}, ctx.Err()
	}
	if err := c.Output.Stdout(ctx, request); err != nil {
		return commandwire.Completion{}, err
	}
	for index := 0; index < 3; index++ {
		if index == 2 {
			select {
			case <-p.reuse:
			case <-ctx.Done():
				return commandwire.Completion{}, ctx.Err()
			}
			// The original request has now been answered; its ID may be reused.
			if err := c.Output.Stdout(ctx, request); err != nil {
				return commandwire.Completion{}, err
			}
		}
		b, err := c.Input.Next(ctx)
		if err != nil {
			return commandwire.Completion{}, err
		}
		answer, err := commandwire.DecodeNumbersAnswer(b)
		if err != nil {
			return commandwire.Completion{}, err
		}
		select {
		case p.replies <- answer:
		case <-ctx.Done():
			return commandwire.Completion{}, ctx.Err()
		}
	}
	return commandwire.Completion{}, nil
}

func TestNumbersRejectDuplicateInFlightIDAndAllowReuse(t *testing.T) {
	for _, queued := range []bool{false, true} {
		t.Run(fmt.Sprint("queued=", queued), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				duplicate := make(chan struct{})
				reuse := make(chan struct{})
				replies := make(chan commandwire.NumbersAnswer)
				c, _ := connected(t, &numberPeer{duplicate: duplicate, reuse: reuse, replies: replies})
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				input, output, err := c.Invoke(ctx, invocation("numbers-peer"))
				must(t, err)
				stream := &RequestStream{input: input, output: output}
				started := make(chan struct{})
				release := make(chan struct{})
				var calls atomic.Int32
				done := make(chan error, 1)
				go func() {
					done <- stream.AnswerNumbers(ctx, func(ctx context.Context, _ commandwire.NumbersRequest) (uint64, error) {
						call := calls.Add(1)
						if call == 1 {
							close(started)
							select {
							case <-release:
							case <-ctx.Done():
								return 0, ctx.Err()
							}
						}
						return uint64(call), nil
					})
					close(done)
				}()
				defer func() {
					cancel()
					<-done
				}()
				<-started
				if queued {
					close(release)
					synctest.Wait()
				}
				close(duplicate)
				for index := 0; index < 2; index++ {
					answer := <-replies
					refusal := index == 0
					if queued {
						refusal = index == 1
					}
					if answer.ID != 7 {
						t.Fatalf("reply ID %d", answer.ID)
					}
					if refusal {
						if answer.Error == nil || !strings.Contains(*answer.Error, "duplicate numbers request id") {
							t.Fatalf("duplicate was not refused: %+v", answer)
						}
						if calls.Load() != 1 {
							t.Fatalf("duplicate reached callback: %d calls", calls.Load())
						}
						if !queued {
							close(release)
						}
					} else if answer.First == nil || *answer.First != 1 {
						t.Fatalf("original answer lost: %+v", answer)
					}
				}
				close(reuse)
				answer := <-replies
				if answer.First == nil || *answer.First != 2 || answer.ID != 7 {
					t.Fatalf("ID reuse failed: %+v", answer)
				}
				must(t, <-done)
				if calls.Load() != 2 {
					t.Fatalf("callback count %d, want 2", calls.Load())
				}
			})
		})
	}
}
