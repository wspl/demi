package hostremote_test

import (
	"context"
	"io"
	"strings"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/hostremote"
	"github.com/wspl/demi/go/hostremote/hostremotetest"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
	"github.com/wspl/demi/go/shell/shelltest"
)

type heldKeeper struct {
	entered chan *shell.WholeOutput
	release chan struct{}
}

func (k *heldKeeper) Retain(context.Context, core.CommandID, []runnerproto.JobFileChange) []core.EditedFile {
	return nil
}
func (k *heldKeeper) KeepOutput(_ context.Context, _ core.CommandID, output *shell.WholeOutput) {
	k.entered <- output
	<-k.release
}

// Cost: fake runner and fake time. The observation clock advances only after
// job_start and its output have arrived. Publication waits on an explicit keeper.
func TestEnvironmentObservesWholeLinesAndPublishesBeforeRelease(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		device := hostremotetest.NewTestDevice(t, nil)
		link := device.Connect(0)
		pages := shelltest.NewPages(true)
		keeper := &heldKeeper{entered: make(chan *shell.WholeOutput, 1), release: make(chan struct{})}
		options := hostremote.NewEnvironmentOptions(device.Host("/work", nil), device.Execute, func(context.Context) (commandservice.CommandContext, error) { return shelltest.CommandContext(), nil }, pages, &shelltest.CountingNumbers{})
		options.Keeper = keeper
		env := hostremote.NewRemoteShellEnvironment(options)
		var callers sync.WaitGroup
		defer func() {
			select {
			case <-keeper.release:
			default:
				close(keeper.release)
			}
			link.Link.Disconnect("test over")
			env.DisposeAll(context.Background())
			callers.Wait()
		}()
		window, _ := shell.NewObservationWindow(1000)
		result := make(chan shell.CommandStatus, 1)
		callers.Add(1)
		go func() {
			defer callers.Done()
			status, err := env.Exec(t.Context(), shell.ExecRequest{Script: "script", Caller: caller(t), Window: window})
			if err != nil {
				t.Error(err)
			}
			result <- status
		}()
		start := next(t, link).(runnerproto.InboundJobStart)
		if follow, ok := next(t, link).(runnerproto.InboundJobFollow); !ok || !follow.Follow {
			t.Fatal("watched job not followed")
		}
		send(t, link, runnerproto.OutboundJobOutput{JobID: start.JobID, Stream: runnerproto.OutputStreamStdout, Bytes: []byte("line\npartial")})
		for {
			page, err := pages.Next(t.Context())
			if err != nil {
				t.Fatal(err)
			}
			if page.Tail == "line\npartial" {
				break
			}
		}
		time.Sleep(time.Second)
		status := <-result
		if status.State.Phase != shell.CommandRunning || status.Stdout.Delta != "line\npartial" {
			t.Fatalf("first look: %+v", status)
		}
		again, err := env.Status(status.CommandID)
		if err != nil || again.Output.Text != "partial" {
			t.Fatalf("partial line skipped: %+v %v", again, err)
		}
		send(t, link, runnerproto.OutboundJobOutput{JobID: start.JobID, Stream: runnerproto.OutputStreamStdout, Offset: 40000})
		send(t, link, runnerproto.OutboundJobExit{JobID: start.JobID, ExitCode: new(int32(0)), Output: &runnerproto.OutputLengths{StdoutBytes: 40000}, Files: []runnerproto.JobFileChange{}})
		read := next(t, link).(runnerproto.InboundJobRead)
		data, err := hostremote.EncodeOutput(shell.NewWholeOutput([]shell.OutputRecord{shell.OutputRead{Stream: core.StreamKindStdout, Bytes: []byte(strings.Repeat("x", 40000))}}, nil))
		if err != nil {
			t.Fatal(err)
		}
		source, err := device.Pipes.ClaimSource(read.Output.ID, hostremotetest.TestDeviceID)
		if err != nil {
			t.Fatal(err)
		}
		sent := make(chan error, 1)
		go func() { sent <- source.Pump(t.Context(), io.NopCloser(strings.NewReader(string(data)))) }()
		send(t, link, runnerproto.OutboundJobRead{ID: read.ID})
		kept := <-keeper.entered
		if len(kept.Text(shell.Streams{}, nil, shell.Seen{}).Bytes()) != 40000 {
			t.Fatal("wrong kept records")
		}
		waiting, err := env.Status(status.CommandID)
		if err != nil || waiting.State.Phase != shell.CommandRunning {
			t.Fatal("published before keeper", waiting.State, err)
		}
		if _, available, err := link.TryNext(); err != nil || available {
			t.Fatal("released before keeper", err)
		}
		close(keeper.release)
		if release, ok := next(t, link).(runnerproto.InboundJobRelease); !ok || release.JobID != start.JobID {
			t.Fatal("job not released")
		}
		for {
			page, err := pages.Next(t.Context())
			if err != nil {
				t.Fatal(err)
			}
			if page.State.Phase == shell.CommandExited {
				break
			}
		}
		final, err := env.Status(status.CommandID)
		if err != nil || final.Whole == nil || final.State.ExitCode != 0 {
			t.Fatal(final.State, err)
		}
		if err := <-sent; err != nil {
			t.Fatal(err)
		}
	})
}

// Cost: three fake jobs. Starts, page publication, and exit replies are barriers;
// the ten-minute observation is only a hang guard.
func TestEnvironmentAllocatesBusyShellsAndDisposesWithCancelledContext(t *testing.T) {
	device := hostremotetest.NewTestDevice(t, nil)
	link := device.Connect(0)
	pages := shelltest.NewPages(false)
	options := hostremote.NewEnvironmentOptions(device.Host("/work", nil), device.Execute, func(context.Context) (commandservice.CommandContext, error) { return shelltest.CommandContext(), nil }, pages, &shelltest.CountingNumbers{})
	env := hostremote.NewRemoteShellEnvironment(options)
	var callers sync.WaitGroup
	defer func() {
		link.Link.Disconnect("test over")
		env.DisposeAll(context.Background())
		callers.Wait()
	}()
	window, _ := shell.NewObservationWindow(600000)
	results := make(chan shell.CommandStatus, 3)
	launch := func(target shell.ShellTarget) runnerproto.InboundJobStart {
		callers.Add(1)
		go func() {
			defer callers.Done()
			v, err := env.Exec(t.Context(), shell.ExecRequest{Script: "wait", Shell: target, Caller: caller(t), Window: window})
			if err != nil {
				t.Error(err)
			}
			results <- v
		}()
		return next(t, link).(runnerproto.InboundJobStart)
	}
	first := launch(shell.ShellTarget{})
	page, err := pages.Next(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	_, err = env.Exec(t.Context(), shell.ExecRequest{Shell: shell.ShellTarget{Kind: shell.ShellExisting, ID: page.ShellID}, Caller: caller(t), Window: window})
	if err == nil || err.Error() != `Shell session "1" is already running command "1"` {
		t.Fatalf("busy shell: %v", err)
	}
	second := launch(shell.ShellTarget{})
	secondPage, err := pages.Next(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if secondPage.ShellID == page.ShellID || first.Cwd != "/work" || second.Cwd != "/work" {
		t.Fatal("default spill reused busy shell")
	}
	third := launch(shell.ShellTarget{Kind: shell.ShellEphemeral, Cwd: new("/other")})
	if third.Cwd != "/other" {
		t.Fatal("ephemeral cwd")
	}
	views := env.PageViews()
	if len(views) != 3 || views[0].CommandID.String() != "1" || views[1].CommandID.String() != "2" || views[2].CommandID.String() != "3" {
		t.Fatal("page views are not in command order", views)
	}
	cancelled, cancel := context.WithCancel(context.Background())
	cancel()
	disposed := make(chan error, 1)
	go func() { disposed <- env.DisposeAll(cancelled) }()
	for range 3 {
		kill := next(t, link).(runnerproto.InboundJobKill)
		if kill.Signal == nil || *kill.Signal != runnerproto.SignalTerminate {
			t.Fatal(kill)
		}
		send(t, link, runnerproto.OutboundJobExit{JobID: kill.JobID, Signal: new("SIGTERM"), Files: []runnerproto.JobFileChange{}})
		if _, ok := next(t, link).(runnerproto.InboundJobRelease); !ok {
			t.Fatal("not released")
		}
	}
	if err := <-disposed; err != context.Canceled {
		t.Fatal(err)
	}
	for range 3 {
		if result := <-results; result.State.Phase != shell.CommandAborted {
			t.Fatal(result.State)
		}
	}
	if env.OwnsShell(page.ShellID) {
		t.Fatal("disposed shell retained")
	}
}

// Cost: fake time and six small output frames. The reader's idle barrier lets
// each frame reach the environment before checking a view or advancing time.
func TestEnvironmentSeparatesLiveUTF8TailHintsAndLostOutput(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		device := hostremotetest.NewTestDevice(t, nil)
		link := device.Connect(0)
		pages := shelltest.NewPages(true)
		options := hostremote.NewEnvironmentOptions(device.Host("/work", nil), device.Execute, func(context.Context) (commandservice.CommandContext, error) { return shelltest.CommandContext(), nil }, pages, &shelltest.CountingNumbers{})
		env := hostremote.NewRemoteShellEnvironment(options)
		var callers sync.WaitGroup
		defer func() {
			link.Link.Disconnect("test over")
			env.DisposeAll(context.Background())
			callers.Wait()
		}()
		window, _ := shell.NewObservationWindow(600000)
		result := make(chan shell.CommandStatus, 1)
		callers.Add(1)
		go func() {
			defer callers.Done()
			status, err := env.Exec(t.Context(), shell.ExecRequest{Script: "stream", Caller: caller(t), Window: window})
			if err != nil {
				t.Error(err)
			}
			result <- status
		}()
		start := next(t, link).(runnerproto.InboundJobStart)
		next(t, link)
		initial, err := pages.Next(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		send(t, link, runnerproto.OutboundJobOutput{JobID: start.JobID, Stream: runnerproto.OutputStreamStdout, Bytes: []byte{0xe2}})
		send(t, link, runnerproto.OutboundJobOutput{JobID: start.JobID, Stream: runnerproto.OutputStreamStdout, Offset: 1, Bytes: []byte{0x82, 0xac}})
		send(t, link, runnerproto.OutboundJobRunningHint{JobID: start.JobID, InvocationID: "first", Hint: new("first hint")})
		send(t, link, runnerproto.OutboundJobRunningHint{JobID: start.JobID, InvocationID: "second", Hint: new("second hint")})
		synctest.Wait()
		status, err := env.Status(initial.CommandID)
		if err != nil || status.Stdout.Delta != "€" || status.State.Hint == nil || *status.State.Hint != "second hint" {
			t.Fatalf("head or hint: %+v %v", status, err)
		}
		send(t, link, runnerproto.OutboundJobRunningHint{JobID: start.JobID, InvocationID: "second"})
		send(t, link, runnerproto.OutboundJobOutput{JobID: start.JobID, Stream: runnerproto.OutputStreamStdout, Offset: 32768, Bytes: []byte{0xc3}})
		send(t, link, runnerproto.OutboundJobOutput{JobID: start.JobID, Stream: runnerproto.OutputStreamStdout, Offset: 32769, Bytes: []byte{0xa9}})
		synctest.Wait()
		status, err = env.Status(initial.CommandID)
		views := env.PageViews()
		if err != nil || status.Stdout.Delta != "" || status.Output.Text != "€" || status.State.Hint == nil || *status.State.Hint != "first hint" || len(views) != 1 || views[0].Tail != "€\n[... 32765 bytes of stdout not shown ...]\né" {
			t.Fatalf("tail leaked or split: %+v %+v %v", status, views, err)
		}
		time.Sleep(500 * time.Millisecond)
		send(t, link, runnerproto.OutboundJobOutput{JobID: start.JobID, Stream: runnerproto.OutputStreamStdout, Offset: 40000})
		synctest.Wait()
		status, err = env.Status(initial.CommandID)
		if err != nil || status.Unreceived != 39997 || status.IdleMs != 0 {
			t.Fatalf("growth: %+v %v", status, err)
		}
		link.Close()
		final := <-result
		if final.State.Phase != shell.CommandExited || final.State.ExitCode != 127 || final.State.Hint != nil || final.Whole == nil || final.Whole.Output.Missing() == nil || final.Whole.Output.Missing().Bytes != 39997 || final.Whole.Output.Missing().Reason != "lost with the Host's connection" {
			t.Fatalf("lost: %+v", final)
		}
	})
}
