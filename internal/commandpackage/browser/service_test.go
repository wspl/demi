package browser

import (
	"context"
	"errors"
	"io"
	"slices"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/live"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/commandsdk/commandsdktest"
	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

type programSuite struct{ m *testing.M }

// Run builds fixture programs before running the browser suite.
func (s programSuite) Run() int { return programtest.Run(s.m) }

// TestMain checks that the browser suite leaves no goroutines running.
func TestMain(m *testing.M) { goleak.VerifyTestMain(programSuite{m}) }

func invocation(operation, args, conversation string) commandsdk.InvocationContext[commandproto.Invocation] {
	asJSON := true
	return commandsdk.InvocationContext[commandproto.Invocation]{Request: commandproto.Invocation{
		Operation:    "browser." + operation,
		InvocationID: operation,
		Args:         []byte(args),
		Cwd:          "/",
		Env:          map[string]string{},
		JSON:         &asJSON,
		Context: commandproto.Context{
			Conversation: conversation,
			Caller:       &commandproto.AgentCaller{Number: 1},
			Locale:       commandproto.CommandLocale{TimeZone: "UTC", Languages: []commandproto.LanguageTag{"en-US"}},
		},
	}}
}

func call(
	t *testing.T,
	s *service,
	request commandsdk.InvocationContext[commandproto.Invocation],
) (commandproto.Completion, []byte, []byte) {
	t.Helper()
	output, records := commandsdk.OutputChannel(t.Context())
	request.Output = output
	completion, stdout, stderr, err := collectInvocation(t.Context(), s, request, records)
	if err != nil {
		t.Fatal(err)
	}
	return completion, stdout, stderr
}

func callLifecycle(t *testing.T, s *service, request commandproto.ConversationRequest) []byte {
	t.Helper()
	output, records := commandsdk.OutputChannel(t.Context())
	completion, err := s.Conversation(t.Context(), commandsdk.ConversationContext{Request: request, Output: output})
	if err != nil || completion.ExitCode != 0 {
		t.Fatalf("lifecycle: %+v %v", completion, err)
	}
	return []byte((<-records).(commandproto.Stdout))
}

func TestAbsentBrowserAndInputRefusals(t *testing.T) {
	s := newService()
	defer func() {
		if err := s.Close(t.Context()); err != nil {
			t.Error(err)
		}
	}()
	s.launch = func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error) {
		t.Error("listing or invalid input launched Chrome")
		return nil, nil, errors.New("unexpected launch")
	}
	for _, test := range []struct {
		name, args, code string
		exit             uint8
	}{
		{"tabs", "{}", "", 0},
		{"info", `{"tab":"t1"}`, "tab_not_found", 1},
		{"open", `{"url":4}`, "invalid_input", 2},
		{"tabs", `{"limit":0}`, "invalid_input", 2},
		{"live", `{"unknown":true}`, "invalid_input", 2},
	} {
		t.Run(test.name+test.args, func(t *testing.T) {
			completion, stdout, stderr := call(t, s, invocation(test.name, test.args, "one"))
			if completion.ExitCode != test.exit {
				t.Fatalf("completion=%+v stderr=%s", completion, stderr)
			}
			if test.code == "" {
				result, err := browserproto.DecodeTabsResult(stdout)
				if err != nil || len(result.Tabs) != 0 {
					t.Fatalf("tabs=%s err=%v", stdout, err)
				}
			} else {
				result, err := browserproto.DecodeFailureDocument(stderr)
				if err != nil || string(result.Error.Code) != test.code {
					t.Fatalf("failure=%s err=%v", stderr, err)
				}
			}
		})
	}
	if got := string(callLifecycle(t, s, &commandproto.ConversationQuery{})); got != `{"conversations":[]}` {
		t.Fatal(got)
	}
	for range 2 {
		if got := string(callLifecycle(t, s, &commandproto.ConversationRelease{Conversation: "one"})); got != "{}" {
			t.Fatal(got)
		}
	}
}

// A release cancels a command blocked on bounded output; no browser or wall time.
func TestReleaseCancelsBrowserCommandBlockedOnOutput(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newService()
		defer func() {
			if err := s.Close(t.Context()); err != nil {
				t.Error(err)
			}
		}()
		request := invocation("tabs", "{}", "one")
		output, records := commandsdk.OutputChannel(t.Context())
		request.Output = output
		for range 4 {
			if err := output.Stdout(t.Context(), []byte("occupied")); err != nil {
				t.Fatal(err)
			}
		}
		done := make(chan error, 1)
		go func() {
			_, err := s.Invoke(t.Context(), request)
			done <- err
		}()
		synctest.Wait()
		select {
		case err := <-done:
			t.Fatalf("command was not blocked: %v", err)
		default:
		}
		if got := string(callLifecycle(t, s, &commandproto.ConversationRelease{Conversation: "one"})); got != "{}" {
			t.Fatal(got)
		}
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("blocked command: %v", err)
		}
		for range 4 {
			if string((<-records).(commandproto.Stdout)) != "occupied" {
				t.Fatal("output changed")
			}
		}
		select {
		case record := <-records:
			t.Fatalf("cancelled command wrote output: %v", record)
		default:
		}
		completion, _, stderr := call(t, s, invocation("tabs", "{}", "two"))
		if completion.ExitCode != 0 {
			t.Fatalf("other conversation: %s", stderr)
		}
	})
}

func TestStartupJoinsAndStatusDoesNotWait(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newService()
		defer func() {
			if err := s.Close(t.Context()); err != nil {
				t.Error(err)
			}
		}()
		var starts atomic.Int32
		release := make(chan struct{})
		s.launch = func(ctx context.Context, start *starting, _ tabs.NumberSource) (*tabs.Environment, *live.Hub, error) {
			starts.Add(1)
			if start.locale.TimeZone != "UTC" || start.invocation != "open" {
				t.Error("untrusted launch metadata")
			}
			select {
			case <-release:
				return nil, nil, &cdp.BrowserError{Kind: cdp.KindUnavailable, Message: "fixture startup failure"}
			case <-ctx.Done():
				return nil, nil, ctx.Err()
			}
		}
		done := make(chan commandproto.Completion, 2)
		for range 2 {
			go func() {
				completion, _, _ := call(t, s, invocation("open", `{"url":"about:blank"}`, "one"))
				done <- completion
			}()
		}
		synctest.Wait()
		if starts.Load() != 1 {
			t.Fatalf("concurrent starts=%d", starts.Load())
		}
		if got := string(callLifecycle(t, s, &commandproto.ConversationQuery{})); got != `{"conversations":["one"]}` {
			t.Fatal(got)
		}
		completion, _, stderr := call(t, s, invocation("tabs", "{}", "two"))
		if completion.ExitCode != 0 {
			t.Fatalf("other conversation: %s", stderr)
		}
		close(release)
		for range 2 {
			if (<-done).ExitCode != 1 {
				t.Fatal("startup did not fail")
			}
		}
		synctest.Wait()
		completion, _, _ = call(t, s, invocation("open", `{"url":"about:blank"}`, "one"))
		if completion.ExitCode != 1 || starts.Load() != 2 {
			t.Fatal("failed startup did not allow a fresh explicit open")
		}
	})
}

func TestReleaseDuringStartupCancelsOnlyItsCommands(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newService()
		defer func() {
			if err := s.Close(t.Context()); err != nil {
				t.Error(err)
			}
		}()
		started := make(chan struct{})
		finished := make(chan struct{})
		s.launch = func(ctx context.Context, _ *starting, _ tabs.NumberSource) (*tabs.Environment, *live.Hub, error) {
			close(started)
			<-ctx.Done()
			close(finished)
			return nil, nil, ctx.Err()
		}
		request := invocation("open", `{"url":"about:blank"}`, "one")
		request.Output, _ = commandsdk.OutputChannel(t.Context())
		done := make(chan error, 1)
		go func() {
			_, err := s.Invoke(t.Context(), request)
			done <- err
		}()
		<-started
		callLifecycle(t, s, &commandproto.ConversationRelease{Conversation: "one"})
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
		<-finished
		completion, _, _ := call(t, s, invocation("tabs", "{}", "two"))
		if completion.ExitCode != 0 {
			t.Fatal(completion)
		}
		if got := string(callLifecycle(t, s, &commandproto.ConversationQuery{})); got != `{"conversations":[]}` {
			t.Fatal(got)
		}
	})
}

func TestCommandDeadlineDoesNotCancelSharedStartup(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newService()
		defer func() {
			if err := s.Close(t.Context()); err != nil {
				t.Error(err)
			}
		}()
		s.launch = func(ctx context.Context, _ *starting, _ tabs.NumberSource) (*tabs.Environment, *live.Hub, error) {
			<-ctx.Done()
			return nil, nil, ctx.Err()
		}
		before := time.Now()
		completion, _, stderr := call(t, s, invocation("open", `{"url":"about:blank","timeout":10}`, "one"))
		failure, err := browserproto.DecodeFailureDocument(stderr)
		if err != nil || failure.Error.Code != "timeout" || completion.ExitCode != 1 {
			t.Fatalf("%s %v", stderr, err)
		}
		if time.Since(before) != 10*time.Millisecond {
			t.Fatal("wrong command deadline")
		}
		if got := string(callLifecycle(t, s, &commandproto.ConversationQuery{})); got != `{"conversations":["one"]}` {
			t.Fatal(got)
		}
		callLifecycle(t, s, &commandproto.ConversationRelease{Conversation: "one"})
	})
}

// The shared incremental executable build dominates this otherwise local wire scenario.
func TestResidentProgramListsWithoutChromeAndShutsDown(t *testing.T) {
	path, err := programtest.Path(t.Context(), "demi-browser")
	if err != nil {
		t.Fatal(err)
	}
	process, err := commandsdktest.Start(t.Context(), t, path, []string{"--command-service"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	// The runner keeps its artifact stream open for the startup sweep, even
	// when no invocation needs an installed browser.
	ctx, cancel := context.WithCancel(t.Context())
	artifacts, err := process.Client.Artifacts(ctx)
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	answered := make(chan error, 1)
	go func() {
		answered <- artifacts.AnswerArtifacts(ctx, func(
			_ context.Context,
			q commandproto.ArtifactRequest,
		) (commandproto.ArtifactAnswer, error) {
			if q.Install != nil {
				return commandproto.ArtifactAnswer{}, errors.New("tab listing requested a Chrome install")
			}
			installed := []commandproto.InstalledArtifact{}
			return commandproto.ArtifactAnswer{ID: q.ID, Installed: &installed}, nil
		})
		close(answered)
	}()
	t.Cleanup(func() {
		cancel()
		<-answered
	})
	info, err := process.Client.Info(t.Context())
	if err != nil || !slices.Equal(info.Operations, browserproto.OperationNames()) {
		t.Fatalf("info=%+v err=%v", info, err)
	}
	input, output, err := process.Client.Invoke(t.Context(), invocation("tabs", "{}", "one").Request)
	if err != nil {
		t.Fatal(err)
	}
	defer input.Cancel()
	var stdout []byte
	var completed bool
	for {
		record, err := output.Next(t.Context())
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			t.Fatal(err)
		}
		switch r := record.(type) {
		case commandproto.Stdout:
			stdout = append(stdout, r...)
		case commandproto.Completed:
			completed = true
			if r.Completion.ExitCode != 0 {
				t.Fatal(r.Completion)
			}
		case commandproto.InputPull, commandproto.Stderr:
			t.Fatalf("unexpected record %T", record)
		}
	}
	if !completed || string(stdout) != `{"tabs":[],"truncated":false}`+"\n" {
		t.Fatalf("stdout=%q completed=%v", stdout, completed)
	}
	if err := process.Shutdown(t.Context()); err != nil {
		t.Fatal(err)
	}
	if err := <-answered; err != nil {
		t.Fatal(err)
	}
}

func TestCleanupFailureFencesAnotherStart(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newService()
		var starts atomic.Int32
		retained := &cdp.BrowserError{
			Kind:  cdp.KindProfileRetained,
			Path:  "/fixture/profile",
			Cause: errors.New("fixture retirement refused"),
		}
		s.launch = func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error) {
			starts.Add(1)
			return nil, nil, retained
		}
		completion, _, _ := call(t, s, invocation("open", `{"url":"about:blank"}`, "one"))
		if completion.ExitCode != 1 {
			t.Fatal(completion)
		}
		synctest.Wait()
		completion, _, stderr := call(t, s, invocation("open", `{"url":"about:blank"}`, "one"))
		if completion.ExitCode != 1 || starts.Load() != 1 {
			t.Fatalf("unresolved cleanup allowed a restart: %s", stderr)
		}
		if err := s.Close(t.Context()); !errors.Is(err, retained) {
			t.Fatalf("cleanup cause lost: %v", err)
		}
	})
}

func TestShutdownRetiresConversationsTogether(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := newService()
		started := make(chan struct{}, 2)
		canceled := make(chan struct{}, 2)
		join := make(chan struct{})
		s.launch = func(ctx context.Context, _ *starting, _ tabs.NumberSource) (*tabs.Environment, *live.Hub, error) {
			started <- struct{}{}
			<-ctx.Done()
			canceled <- struct{}{}
			<-join
			return nil, nil, ctx.Err()
		}
		commands := make(chan error, 2)
		for _, id := range []string{"one", "two"} {
			request := invocation("open", `{"url":"about:blank"}`, id)
			request.Output, _ = commandsdk.OutputChannel(t.Context())
			go func() {
				_, err := s.Invoke(t.Context(), request)
				commands <- err
			}()
		}
		<-started
		<-started
		closed := make(chan error, 1)
		go func() { closed <- s.Close(t.Context()) }()
		// Both cancellations must arrive before either cleanup may finish.
		<-canceled
		<-canceled
		close(join)
		if err := <-closed; err != nil {
			t.Fatal(err)
		}
		for range 2 {
			if err := <-commands; !errors.Is(err, context.Canceled) {
				t.Fatal(err)
			}
		}
	})
}

func TestViewerNotificationsEndWithConversation(t *testing.T) {
	s := newService()
	b, err := s.admit("one")
	if err != nil {
		t.Fatal(err)
	}
	b.commands.Done()
	before := b.Changed()
	environment, hub, err := b.Running(t.Context())
	if err != nil || environment != nil || hub != nil {
		t.Fatalf("absent browser: %p %p %v", environment, hub, err)
	}
	if err := s.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	select {
	case <-before:
	default:
		t.Fatal("viewer was not notified when owner ended")
	}
	select {
	case <-b.Released():
	default:
		t.Fatal("viewer was not released")
	}
	_, _, err = b.Running(t.Context())
	var failure *cdp.BrowserError
	if !errors.As(err, &failure) || failure.Kind != cdp.KindClosed {
		t.Fatalf("ended owner: %v", err)
	}
}

func TestUserCloseOfExpiredTabIsRefused(t *testing.T) {
	s := newService()
	defer func() {
		if err := s.Close(t.Context()); err != nil {
			t.Error(err)
		}
	}()
	request := invocation("close", `{"tab":"t99"}`, "one")
	request.Request.Context.Caller = &commandproto.UserCaller{}
	completion, stdout, stderr := call(t, s, request)
	result, err := browserproto.DecodeFailureDocument(stderr)
	if err != nil || completion.ExitCode != 1 || result.Error.Code != "tab_not_found" {
		t.Fatalf("close=%s stderr=%s completion=%+v err=%v", stdout, stderr, completion, err)
	}
	request.Request.Context.Caller = &commandproto.AgentCaller{Number: 1}
	completion, _, stderr = call(t, s, request)
	failure, err := browserproto.DecodeFailureDocument(stderr)
	if err != nil || completion.ExitCode != 1 || failure.Error.Code != "tab_not_found" {
		t.Fatalf("agent close=%s err=%v", stderr, err)
	}
}

func TestRefusedArgumentsRetainOnlyStringDiagnosticTab(t *testing.T) {
	s := newService()
	defer func() {
		if err := s.Close(t.Context()); err != nil {
			t.Error(err)
		}
	}()
	for _, test := range []struct {
		args string
		tab  *string
	}{
		{`{"tab":"t1","timeout":"bad"}`, new("t1")},
		{`{"tab":42,"timeout":"bad"}`, nil},
		{`{"tab":null,"timeout":"bad"}`, nil},
	} {
		completion, _, stderr := call(t, s, invocation("info", test.args, "one"))
		failure, err := browserproto.DecodeFailureDocument(stderr)
		if err != nil || completion.ExitCode != 2 || failure.Error.Details == nil {
			t.Fatalf("%s %v", stderr, err)
		}
		got := failure.Error.Details.Tab
		if (got == nil) != (test.tab == nil) || got != nil && *got != *test.tab {
			t.Fatalf("diagnostic tab for %s: %s", test.args, stderr)
		}
	}
}

// collectInvocation drains browser output while joining the invocation that owns it.
func collectInvocation(
	ctx context.Context,
	s *service,
	request commandsdk.InvocationContext[commandproto.Invocation],
	records <-chan commandproto.Record,
) (commandproto.Completion, []byte, []byte, error) {
	type answer struct {
		completion commandproto.Completion
		err        error
	}
	done := make(chan answer, 1)
	go func() {
		c, err := s.Invoke(ctx, request)
		done <- answer{c, err}
	}()
	var stdout, stderr []byte
	appendRecord := func(record commandproto.Record) {
		switch r := record.(type) {
		case commandproto.Stdout:
			stdout = append(stdout, r...)
		case commandproto.Stderr:
			stderr = append(stderr, r...)
		case commandproto.Completed, commandproto.InputPull:
		}
	}
	for {
		select {
		case record := <-records:
			appendRecord(record)
		case result := <-done:
			for {
				select {
				case record := <-records:
					appendRecord(record)
				default:
					return result.completion, stdout, stderr, result.err
				}
			}
		}
	}
}
