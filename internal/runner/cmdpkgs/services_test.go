package cmdpkgs_test

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/commandwire/commandwiretest"
	"github.com/wspl/demi/internal/programtest"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/cmdpkgs/cmdpkgstest"
)

// Service scenarios use a real child and HTTP/2. A cold fixture build costs more
// than one second; programtest shares that single build across the suite.
func fixture(t *testing.T, root string, variant byte) (commandwire.PackageDescriptor, *localResolver) {
	t.Helper()
	path, err := programtest.Path(t.Context(), "demi-native-fixture")
	must(t, err)
	data, err := os.ReadFile(path)
	must(t, err)
	if variant != 0 {
		data = append(data, variant)
	}
	resolver := writeSource(t, root, fmt.Sprintf("fixture-%d", variant), data)
	target, err := commandwire.HostTarget()
	must(t, err)
	return commandwire.PackageDescriptor{
		ID:              "demi.fixture",
		Version:         "1.0.0",
		ProtocolVersion: 1,
		Operations:      commandwiretest.FixtureOperations(),
		Targets:         map[string]commandwire.PackageArtifact{string(target): artifact(data)},
	}, resolver
}

func digest(d commandwire.PackageDescriptor) string {
	for _, a := range d.Targets {
		return a.SHA256
	}
	return ""
}

func registry(t *testing.T, root, image string) *cmdpkgs.ServiceRegistry {
	t.Helper()
	r, err := cmdpkgs.NewServiceRegistry(t.Context(), filepath.Join(root, "cache"), image, root, map[string]string{})
	must(t, err)
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
		defer cancel()
		must(t, r.Close(ctx))
	})
	return r
}

func acquire(
	t *testing.T,
	r *cmdpkgs.ServiceRegistry,
	d commandwire.PackageDescriptor,
	resolver cmdpkgs.ArtifactResolver,
) *cmdpkgs.Resident {
	t.Helper()
	resident, err := r.Acquire(t.Context(), d, resolver, cmdpkgstest.NoNumbers{})
	must(t, err)
	return resident
}

func lease(t *testing.T, r *cmdpkgs.ServiceRegistry, d commandwire.PackageDescriptor) *cmdpkgs.ServiceLease {
	t.Helper()
	l, err := r.Lease(t.Context(), digest(d))
	must(t, err)
	t.Cleanup(l.Release)
	return l
}

func invocation(operation, conversation string) commandwire.Invocation {
	return commandwire.Invocation{
		Operation:    operation,
		InvocationID: operation,
		Context: commandwire.CommandContext{
			Conversation: conversation,
			Caller:       &commandwire.AgentCaller{Number: 1},
			Locale:       commandwire.CommandLocale{TimeZone: "UTC", Languages: []commandwire.LanguageTag{"en-US"}},
		},
		Args: []byte("{}"),
		Cwd:  os.TempDir(),
		Env:  map[string]string{},
	}
}

func call(ctx context.Context, resident *cmdpkgs.Resident, operation, conversation string) (string, error) {
	input, output, err := resident.Client().Invoke(ctx, invocation(operation, conversation))
	if err != nil {
		return "", err
	}
	defer input.Cancel()
	if err := input.End(); err != nil {
		return "", err
	}
	var text strings.Builder
	completed := false
	for {
		record, err := output.Next(ctx)
		if errors.Is(err, io.EOF) {
			if !completed {
				return "", errors.New("invocation ended without completion")
			}
			return text.String(), nil
		}
		if err != nil {
			return "", err
		}
		switch v := record.(type) {
		case commandwire.Stdout:
			text.Write(v)
		case commandwire.Stderr:
		case commandwire.Completed:
			completed = true
			if v.Completion.ExitCode != 0 {
				return "", fmt.Errorf("completion: %+v", v.Completion)
			}
		case commandwire.InputPull:
		}
	}
}

func invoke(t *testing.T, resident *cmdpkgs.Resident, operation, conversation string) string {
	t.Helper()
	text, err := call(t.Context(), resident, operation, conversation)
	must(t, err)
	return text
}

func decided(
	t *testing.T,
	receiver *cmdpkgs.DecisionSubscription,
	d commandwire.PackageDescriptor,
	want cmdpkgs.Decision,
) {
	t.Helper()
	event, err := receiver.Next(t.Context())
	must(t, err)
	if event.Digest != digest(d) || event.Decision != want {
		t.Fatalf("decision=%+v; want %v", event, want)
	}
}

func stopped(t *testing.T, resident *cmdpkgs.Resident) {
	t.Helper()
	err := resident.Failure(t.Context(), io.EOF)
	var failure *cmdpkgs.RuntimeError
	if !errors.As(err, &failure) || failure.Kind != cmdpkgs.Stopped {
		t.Fatalf("stop: %v", err)
	}
	if _, err := resident.Client().Info(t.Context()); err == nil {
		t.Fatal("stopped client still answers")
	}
}

func TestServiceWithoutLeasesStaysWhileHoldingConversation(t *testing.T) {
	root := t.TempDir()
	r := registry(t, root, "")
	d, resolver := fixture(t, root, 0)
	events := r.Decisions(t.Context())
	l := lease(t, r, d)
	resident := acquire(t, r, d, resolver)
	decided(t, events, d, cmdpkgs.Leased)
	invoke(t, resident, "retain", "one")
	invoke(t, resident, "retain", "two")
	l.Release()
	for _, conversation := range []string{"", "unknown", "one"} {
		if conversation != "" {
			must(t, r.ReleaseConversation(t.Context(), conversation))
		}
		decided(t, events, d, cmdpkgs.Asks)
		decided(t, events, d, cmdpkgs.HoldsConversations)
	}
	_, err := resident.Client().Info(t.Context())
	must(t, err)
	must(t, r.ReleaseConversation(t.Context(), "two"))
	decided(t, events, d, cmdpkgs.Asks)
	decided(t, events, d, cmdpkgs.Stops)
	stopped(t, resident)
	must(t, r.ReleaseConversation(t.Context(), "two"))
}

func TestLeaseKeepsServiceHoldingNothing(t *testing.T) {
	root := t.TempDir()
	r := registry(t, root, "")
	d, resolver := fixture(t, root, 0)
	events := r.Decisions(t.Context())
	l := lease(t, r, d)
	resident := acquire(t, r, d, resolver)
	decided(t, events, d, cmdpkgs.Leased)
	_, infoErr := resident.Client().Info(t.Context())
	must(t, infoErr)
	again := acquire(t, r, d, resolver)
	if again.Client() != resident.Client() {
		t.Fatal("did not reuse client")
	}
	invoke(t, again, "retain", "shared")
	must(t, r.ReleaseConversation(t.Context(), "shared"))
	_, err := resident.Client().Info(t.Context())
	must(t, err)
	if resolver.calls.Load() != 1 {
		t.Fatal("repeated download")
	}
	l.Release()
	stopped(t, resident)
}

func TestUnanswerableServiceStays(t *testing.T) {
	root := t.TempDir()
	r := registry(t, root, "")
	d, resolver := fixture(t, root, 0)
	events := r.Decisions(t.Context())
	l := lease(t, r, d)
	resident := acquire(t, r, d, resolver)
	decided(t, events, d, cmdpkgs.Leased)
	invoke(t, resident, "retain", "unanswerable")
	l.Release()
	decided(t, events, d, cmdpkgs.Asks)
	decided(t, events, d, cmdpkgs.Unanswered)
	_, err := resident.Client().Info(t.Context())
	must(t, err)
	must(t, r.ReleaseConversation(t.Context(), "unanswerable"))
	decided(t, events, d, cmdpkgs.Asks)
	decided(t, events, d, cmdpkgs.Stops)
	stopped(t, resident)
}

func TestAnswerBeforeReleaseIsAskedAgain(t *testing.T) {
	root := t.TempDir()
	r := registry(t, root, "")
	d, resolver := fixture(t, root, 0)
	events := r.Decisions(t.Context())
	l := lease(t, r, d)
	resident := acquire(t, r, d, resolver)
	decided(t, events, d, cmdpkgs.Leased)
	invoke(t, resident, "retain", "stall")
	l.Release()
	decided(t, events, d, cmdpkgs.Asks)
	invoke(t, resident, "stalled", "test")
	must(t, r.ReleaseConversation(t.Context(), "stall"))
	invoke(t, resident, "proceed", "test")
	decided(t, events, d, cmdpkgs.Asks)
	decided(t, events, d, cmdpkgs.Stops)
	stopped(t, resident)
}

func TestFailedReleaseRetiresBeforeAnswerAndNextCallerStartsAgain(t *testing.T) {
	root := t.TempDir()
	r := registry(t, root, "")
	d, resolver := fixture(t, root, 0)
	lease(t, r, d)
	resident := acquire(t, r, d, resolver)
	if err := r.ReleaseConversation(t.Context(), "fail"); err == nil {
		t.Fatal("failed release succeeded")
	}
	if _, err := resident.Client().Info(t.Context()); err == nil {
		t.Fatal("release answered before retirement")
	}
	next := acquire(t, r, d, resolver)
	_, err := next.Client().Info(t.Context())
	must(t, err)
}

func TestDeadServiceReportsStatusAndStderr(t *testing.T) {
	root := t.TempDir()
	r := registry(t, root, "")
	d, resolver := fixture(t, root, 0)
	lease(t, r, d)
	resident := acquire(t, r, d, resolver)
	_, err := call(t.Context(), resident, "crash", "c")
	if err == nil {
		t.Fatal("crashing invocation completed")
	}
	failure := resident.Failure(t.Context(), err)
	var exit *cmdpkgs.ServiceExit
	if !errors.As(failure, &exit) || exit.Reason.State.Code == nil || *exit.Reason.State.Code != 3 ||
		!strings.Contains(exit.Stderr, "fixture crashing on purpose") {
		t.Fatalf("failure: %v", failure)
	}
	wantStatus := "exit status: 3"
	if runtime.GOOS == "windows" {
		wantStatus = "exit code: 3"
	}
	if want := "native service demi.fixture exited with " + wantStatus +
		"; its standard error ended with:\nfixture crashing on purpose"; failure.Error() != want {
		t.Fatalf("failure text = %q; want %q", failure, want)
	}
	next := acquire(t, r, d, resolver)
	invoke(t, next, "retain", "after")
}

func TestStartNobodyWaitsForStops(t *testing.T) {
	root := t.TempDir()
	r := registry(t, root, "")
	d, resolver := fixture(t, root, 0)
	gate := make(chan struct{})
	resolver.gate = gate
	resolver.entered = make(chan struct{}, 2)
	resolver.cancelled = make(chan struct{}, 2)
	events := r.Decisions(t.Context())
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := r.Acquire(ctx, d, resolver, cmdpkgstest.NoNumbers{})
		done <- err
	}()
	<-resolver.entered
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatalf("cancel: %v", err)
	}
	decided(t, events, d, cmdpkgs.Stops)
	<-resolver.cancelled
	second := make(chan error, 1)
	l := lease(t, r, d)
	go func() {
		resident, err := r.Acquire(t.Context(), d, resolver, cmdpkgstest.NoNumbers{})
		if err == nil {
			_, err = resident.Client().Info(t.Context())
		}
		second <- err
	}()
	<-resolver.entered
	close(gate)
	must(t, <-second)
	if resolver.calls.Load() != 2 {
		t.Fatalf("calls=%d", resolver.calls.Load())
	}
	l.Release()
}

func TestStopAllAndCloseEndServices(t *testing.T) {
	root := t.TempDir()
	r := registry(t, root, "")
	var residents []*cmdpkgs.Resident
	for variant := byte(0); variant < 2; variant++ {
		d, resolver := fixture(t, root, variant)
		lease(t, r, d)
		resident := acquire(t, r, d, resolver)
		invoke(t, resident, "retain", "held")
		residents = append(residents, resident)
	}
	must(t, r.StopAll(t.Context()))
	for _, resident := range residents {
		if _, err := resident.Client().Info(t.Context()); err == nil {
			t.Fatal("stopped service still answers")
		}
		stopped(t, resident)
	}
	must(t, r.Close(t.Context()))
	d, resolver := fixture(t, root, 0)
	if _, err := r.Acquire(t.Context(), d, resolver, cmdpkgstest.NoNumbers{}); err == nil {
		t.Fatal("closed registry acquired")
	}
}

func preinstall(t *testing.T, image string, d commandwire.PackageDescriptor, data []byte) {
	t.Helper()
	directory := filepath.Join(image, digest(d))
	must(t, os.MkdirAll(directory, 0o700))
	must(t, os.WriteFile(filepath.Join(directory, "fixture"), data, 0o755))
}

func TestServiceStartsFromImageWithoutDownload(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Cloud images are Linux-only; Windows does not inspect them")
	}
	root := t.TempDir()
	d, resolver := fixture(t, root, 0)
	image := filepath.Join(root, "image")
	data, err := os.ReadFile(resolver.path)
	must(t, err)
	preinstall(t, image, d, data)
	r := registry(t, root, image)
	lease(t, r, d)
	resident := acquire(t, r, d, resolver)
	_, err = resident.Client().Info(t.Context())
	must(t, err)
	if resolver.calls.Load() != 0 {
		t.Fatal("image hit asked resolver")
	}
	if _, err := os.Stat(filepath.Join(root, "cache", digest(d))); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("image was copied into cache")
	}
}

func TestMissingOrDamagedImageDownloads(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Cloud images are Linux-only; Windows does not inspect them")
	}
	root := t.TempDir()
	damaged, resolver := fixture(t, root, 0)
	absent, other := fixture(t, root, 1)
	image := filepath.Join(root, "image")
	info, err := os.Stat(resolver.path)
	must(t, err)
	preinstall(t, image, damaged, make([]byte, info.Size()))
	logs := &logCapture{}
	previous := slog.Default()
	slog.SetDefault(slog.New(logs))
	defer slog.SetDefault(previous)
	r := registry(t, root, image)
	for _, item := range []struct {
		d        commandwire.PackageDescriptor
		resolver *localResolver
	}{{damaged, resolver}, {absent, other}} {
		lease(t, r, item.d)
		acquire(t, r, item.d, item.resolver)
		if item.resolver.calls.Load() != 1 {
			t.Fatal("missing/damaged image did not download")
		}
	}
	must(t, r.Close(t.Context()))
	logs.mu.Lock()
	defer logs.mu.Unlock()
	damagedLogs := 0
	for _, record := range logs.lines {
		text := record.Message
		hasSource := false
		record.Attrs(func(a slog.Attr) bool {
			hasSource = hasSource || a.Key == "source"
			return true
		})
		if strings.Contains(text, filepath.Join(image, digest(damaged))) {
			damagedLogs++
			if hasSource || !strings.Contains(text, "does not match its declared SHA-256") {
				t.Fatalf("diagnostic: %s", text)
			}
		}
		if strings.Contains(text, digest(absent)) {
			t.Fatalf("logged absent image: %s", text)
		}
	}
	if damagedLogs != 1 {
		t.Fatalf("damaged image logs=%d", damagedLogs)
	}
}

type logCapture struct {
	mu    sync.Mutex
	lines []slog.Record
}

func (*logCapture) Enabled(context.Context, slog.Level) bool { return true }
func (l *logCapture) Handle(_ context.Context, r slog.Record) error {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.lines = append(l.lines, r.Clone())
	return nil
}
func (l *logCapture) WithAttrs([]slog.Attr) slog.Handler { return l }
func (l *logCapture) WithGroup(string) slog.Handler      { return l }

func TestNativeCallsAreNeverTurnedAway(t *testing.T) {
	root := t.TempDir()
	r := registry(t, root, "")
	d, resolver := fixture(t, root, 0)
	lease(t, r, d)
	resident := acquire(t, r, d, resolver)
	type pending struct {
		input  *cmdsdk.CommandInput
		output *cmdsdk.CommandOutput
	}
	var held []pending
	open := func() pending {
		t.Helper()
		i, o, err := resident.Client().Invoke(t.Context(), invocation("echo", "running"))
		must(t, err)
		t.Cleanup(i.Cancel)
		return pending{i, o}
	}
	for range 128 {
		held = append(held, open())
	}
	for attempt := range 40 {
		held[0].input.Cancel()
		held = held[1:]
		must(t, r.ReleaseConversation(t.Context(), fmt.Sprintf("archived-%d", attempt)))
		held = append(held, open())
	}
	for _, p := range held {
		must(t, p.input.End())
		completed := false
		for {
			record, err := p.output.Next(t.Context())
			if errors.Is(err, io.EOF) {
				break
			}
			must(t, err)
			if v, ok := record.(commandwire.Completed); ok && v.Completion.ExitCode == 0 {
				completed = true
			}
		}
		if !completed {
			t.Fatal("invocation lost completion")
		}
	}
	_, err := resident.Client().Info(t.Context())
	must(t, err)
}

// Lifecycle logs are consumed by the Host log and real-runner scenarios. This
// uses the existing native fixture and waits for the service's end, without polling.
func TestServiceLifecycleMessages(t *testing.T) {
	logs := &logCapture{}
	previous := slog.Default()
	slog.SetDefault(slog.New(logs))
	defer slog.SetDefault(previous)
	root := t.TempDir()
	r := registry(t, root, "")
	d, resolver := fixture(t, root, 0)
	held := lease(t, r, d)
	resident := acquire(t, r, d, resolver)
	held.Release()
	_ = resident.Failure(t.Context(), io.EOF)
	must(t, r.Close(t.Context()))
	logs.mu.Lock()
	defer logs.mu.Unlock()
	var lifecycle []string
	for _, line := range logs.lines {
		if strings.HasPrefix(line.Message, "service "+d.ID+" ") {
			lifecycle = append(lifecycle, line.Message)
		}
	}
	if len(lifecycle) != 3 || !strings.HasPrefix(lifecycle[0], "service "+d.ID+" started (pid ") ||
		!strings.HasSuffix(lifecycle[0], ")") ||
		lifecycle[1] != "service "+d.ID+" holds no lease or conversation and stops" ||
		lifecycle[2] != "service "+d.ID+" stopped" {
		t.Fatalf("lifecycle: %q", lifecycle)
	}
}
