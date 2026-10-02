package remotehost_test

import (
	"bytes"
	"context"
	"errors"
	"log/slog"
	"reflect"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerwire"
)

// shellFixture observes commands through the same page feed the product uses.
func shellFixture(t *testing.T, configure func(*remotehost.EnvironmentOptions)) (*remotehosttest.TestDevice, *remotehosttest.TestLink, *remotehost.ShellEnvironment, *hosttest.Pages) {
	t.Helper()
	d, l, h := linkDevice(t)
	pages := hosttest.NewPages(false)
	options := remotehost.NewEnvironmentOptions(h, func(context.Context) (commandwire.CommandContext, error) { return hosttest.CommandContext(), nil }, pages, &hosttest.CountingNumbers{})
	if configure != nil {
		configure(&options)
	}
	shell := remotehost.NewShellEnvironment(options)
	t.Cleanup(func() {
		_, err := l.Close(context.Background())
		requirePipe(t, err)
		requirePipe(t, shell.DisposeAll(context.Background()))
	})
	return d, l, shell, pages
}

// shellExec uses the Rust scenario's one-millisecond observation window in virtual time.
func shellExec(t *testing.T, s *remotehost.ShellEnvironment, script string) host.CommandStatus {
	t.Helper()
	window, _ := host.NewObservationWindow(1)
	result, err := s.Exec(t.Context(), host.ExecRequest{Script: script, Window: window, Caller: host.JobCaller{Node: "test-session"}, ToolUseID: "call"})
	requirePipe(t, err)
	return result
}

// page waits for a published view, avoiding any scheduler polling.
func page(t *testing.T, p *hosttest.Pages) host.PageView {
	t.Helper()
	view, err := p.Next(t.Context())
	requirePipe(t, err)
	return view
}

// terminalPage waits through output publications to command settlement.
func terminalPage(t *testing.T, p *hosttest.Pages) host.PageView {
	t.Helper()
	for {
		v := page(t, p)
		if v.State.Phase != host.Running {
			return v
		}
	}
}

// outputFrame reports one stream view and its absolute offset.
func outputFrame(t *testing.T, l *remotehosttest.TestLink, id string, offset uint64, data []byte) {
	t.Helper()
	sendFrame(t, l, &runnerwire.JobOutput{JobID: id, Stream: runnerwire.Stdout, Offset: offset, Bytes: data})
}

// answerOutput fills the runner pipe requested by the shell's kept-output read.
func answerOutput(t *testing.T, d *remotehosttest.TestDevice, l *remotehosttest.TestLink, data []byte) {
	t.Helper()
	request := nextFrame(t, l).(*runnerwire.JobRead)
	sendFrame(t, l, &runnerwire.JobReadReply{ID: request.ID})
	source, err := d.Pipes().ClaimSource(request.Output.ID, remotehosttest.TestDeviceID)
	requirePipe(t, err)
	requirePipe(t, source.Pump(t.Context(), pipeBody{bytes.NewReader(data)}))
}

// Cost: all shell scenarios run under synctest; no real runner or elapsed-time wait.
func TestCharacterSplitAcrossMessagesShowsOnceWhole(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		_, l, s, p := shellFixture(t, nil)
		started := shellExec(t, s, "print")
		page(t, p)
		job := nextFrame(t, l).(*runnerwire.JobStart)
		data := []byte("aé€😀")
		for i, b := range data {
			outputFrame(t, l, job.JobID, uint64(i), []byte{b})
		}
		outputFrame(t, l, job.JobID, uint64(len(data)), []byte{255, '!'})
		var shown string
		for !strings.HasSuffix(shown, "!") {
			shown = page(t, p).Tail
		}
		if shown != "aé€😀�!" {
			t.Fatal(shown)
		}
		status, err := s.Status(started.CommandID)
		requirePipe(t, err)
		if status.Stdout.Tail != shown {
			t.Fatal(status.Stdout)
		}
	})
}

func TestGrowthBeyondViewKeepsCommandFromIdling(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		_, l, s, p := shellFixture(t, nil)
		started := shellExec(t, s, "build")
		page(t, p)
		job := nextFrame(t, l).(*runnerwire.JobStart)
		outputFrame(t, l, job.JobID, 0, bytes.Repeat([]byte{'x'}, runnerwire.JobViewBytes))
		page(t, p)
		time.Sleep(5 * time.Second)
		status, err := s.Status(started.CommandID)
		requirePipe(t, err)
		if status.IdleMs != 5000 {
			t.Fatal(status.IdleMs)
		}
		outputFrame(t, l, job.JobID, runnerwire.JobViewBytes+100, []byte{})
		synctest.Wait()
		status, err = s.Status(started.CommandID)
		requirePipe(t, err)
		if status.IdleMs != 0 || len(status.Stdout.Tail) != 4096 || status.Stdout.Bytes != runnerwire.JobViewBytes+100 || status.Unreceived != 100 {
			t.Fatal(status)
		}
	})
}

func TestRunningCommandHoldsStreamsNewestBytesBeyondView(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		_, l, s, p := shellFixture(t, nil)
		started := shellExec(t, s, "build")
		page(t, p)
		job := nextFrame(t, l).(*runnerwire.JobStart)
		outputFrame(t, l, job.JobID, 0, bytes.Repeat([]byte{'x'}, runnerwire.JobViewBytes))
		page(t, p)
		check := func(offset uint64, data []byte, wantOffset, wantLeft uint64, want string) {
			outputFrame(t, l, job.JobID, offset, data)
			synctest.Wait()
			status, err := s.Status(started.CommandID)
			requirePipe(t, err)
			if len(status.Newest) != 1 || status.Newest[0].Offset != wantOffset || status.Newest[0].LeftOut != wantLeft || status.Newest[0].Text != want {
				t.Fatal(status.Newest)
			}
		}
		view := uint64(runnerwire.JobViewBytes)
		check(view+100, []byte("one\ntw"), view+100, 100, "one\ntw")
		check(view+104, []byte("two\nthree\n"), view+100, 100, "one\ntwo\nthree\n")
		far := view * 10
		data := append(bytes.Repeat([]byte{'y'}, runnerwire.JobViewBytes), []byte("end\n")...)
		check(far, data, far+4, far+4-view, string(data[4:]))
	})
}

func TestUnstartedJobEnds127WithRunnerReason(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		_, l, s, p := shellFixture(t, nil)
		started := shellExec(t, s, "true")
		page(t, p)
		job := nextFrame(t, l).(*runnerwire.JobStart)
		reason := "the job was cancelled before it started"
		sendFrame(t, l, &runnerwire.JobExit{JobID: job.JobID, SpawnError: &runnerwire.SpawnError{Kind: runnerwire.SpawnErrorKindOther, Detail: &reason}, Files: []runnerwire.JobFileChange{}})
		terminalPage(t, p)
		status, err := s.Status(started.CommandID)
		requirePipe(t, err)
		if status.State.Phase != host.Exited || status.State.ExitCode != 127 || status.Whole == nil {
			t.Fatal(status)
		}
		var stderr []byte
		for _, record := range status.Whole.Output.Records {
			if record.Stream == core.StreamKindStderr {
				stderr = append(stderr, record.Bytes...)
			}
		}
		if string(stderr) != reason+"\n" {
			t.Fatal(string(stderr))
		}
	})
}

func TestUnreadOutputEndsWithNewestRunnerBytes(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		_, l, s, p := shellFixture(t, nil)
		started := shellExec(t, s, "build")
		page(t, p)
		job := nextFrame(t, l).(*runnerwire.JobStart)
		head := bytes.Repeat([]byte{'x'}, runnerwire.JobViewBytes)
		length := uint64(runnerwire.JobViewBytes * 4)
		newest := []byte("error: the build failed\n")
		outputFrame(t, l, job.JobID, 0, head)
		outputFrame(t, l, job.JobID, length-uint64(len(newest)), newest)
		sendFrame(t, l, &runnerwire.JobExit{JobID: job.JobID, ExitCode: new(int32(1)), Output: &runnerwire.OutputLengths{StdoutBytes: length}, Files: []runnerwire.JobFileChange{}})
		request := nextFrame(t, l).(*runnerwire.JobRead)
		sendFrame(t, l, &runnerwire.JobReadReply{ID: request.ID, Error: new("the job's output is gone")})
		terminalPage(t, p)
		status, err := s.Status(started.CommandID)
		requirePipe(t, err)
		want := []host.OutputRecord{{Stream: core.StreamKindStdout, Bytes: head}, {LeftOut: new(length - runnerwire.JobViewBytes - uint64(len(newest)))}, {Stream: core.StreamKindStdout, Bytes: newest}}
		if status.Whole == nil || !reflect.DeepEqual(status.Whole.Output.Records, want) || status.Whole.Output.Missing != nil {
			t.Fatal(status.Whole)
		}
	})
}

func TestInputWrittenWhileAcquiringHostReachesStartedJob(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		acquired := make(chan struct{})
		_, l, s, p := shellFixture(t, func(o *remotehost.EnvironmentOptions) {
			o.Context = func(ctx context.Context) (commandwire.CommandContext, error) {
				select {
				case <-acquired:
					return hosttest.CommandContext(), nil
				case <-ctx.Done():
					return commandwire.CommandContext{}, ctx.Err()
				}
			}
		})
		started := shellExec(t, s, "read name; echo $name")
		if page(t, p).State.Phase != host.Running {
			t.Fatal("command not shown during acquisition")
		}
		done := make(chan error, 1)
		go func() { done <- s.Write(t.Context(), started.CommandID, []byte("Ana\n")) }()
		synctest.Wait()
		select {
		case err := <-done:
			t.Fatal("write did not wait", err)
		default:
		}
		close(acquired)
		_ = nextFrame(t, l).(*runnerwire.JobStart)
		requirePipe(t, <-done)
		frame := nextFrame(t, l).(*runnerwire.JobStdin)
		if string(frame.Bytes) != "Ana\n" {
			t.Fatal(frame)
		}
	})
}

func TestConversationReleaseWaitsAndAdmitsNoWork(t *testing.T) {
	d := remotehosttest.NewTestDevice(t, remotehosttest.NewCommandPolicy(nil))
	h := d.Host("/work", func() (*gates.Lease, error) {
		t.Error("release attempted admission")
		return nil, errors.New("admission")
	})
	requirePipe(t, h.ReleaseConversation(t.Context(), "conversation"))
	l := d.Connect(nil)
	done := make(chan error, 1)
	for _, failure := range []*string{nil, new("cleanup failed"), new("")} {
		go func() { done <- h.ReleaseConversation(t.Context(), "conversation") }()
		request := nextFrame(t, l).(*runnerwire.ConversationRelease)
		if request.ConversationID != "conversation" {
			t.Fatal(request)
		}
		select {
		case err := <-done:
			t.Fatal("release finished early", err)
		default:
		}
		sendFrame(t, l, &runnerwire.ConversationReleased{ID: request.ID, Error: failure})
		err := <-done
		if failure == nil {
			requirePipe(t, err)
		} else {
			var result *host.Error
			if !errors.As(err, &result) || result.Message != *failure {
				t.Fatal(err)
			}
		}
	}
	go func() { done <- h.ReleaseConversation(t.Context(), "conversation") }()
	nextFrame(t, l)
	_, err := l.Close(t.Context())
	requirePipe(t, err)
	var failure *host.Error
	if !errors.As(<-done, &failure) || failure.Kind != host.Offline {
		t.Fatal(failure)
	}
}

func TestStatusShowsLatestFirstRegisteredHintUntilEnd(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		d, l, s, p := shellFixture(t, nil)
		started := shellExec(t, s, "attend first | attend second")
		page(t, p)
		job := nextFrame(t, l).(*runnerwire.JobStart)
		hint := func(id string, value *string, jobID string) {
			sendFrame(t, l, &runnerwire.JobRunningHint{JobID: jobID, InvocationID: id, Hint: value})
			synctest.Wait()
		}
		shown := func(want *string) {
			status, err := s.Status(started.CommandID)
			requirePipe(t, err)
			if status.State.Phase != host.Running || !reflect.DeepEqual(status.State.Hint, want) {
				t.Fatal(status.State)
			}
		}
		hint("foreign", new("another job"), "not-this-job")
		shown(nil)
		hint("first", new("first hint"), job.JobID)
		hint("second", new("second hint"), job.JobID)
		shown(new("second hint"))
		hint("second", nil, job.JobID)
		shown(new("first hint"))
		hint("first", nil, job.JobID)
		hint("third", new("hint before abort"), job.JobID)
		aborted := make(chan error, 1)
		go func() { aborted <- s.Abort(t.Context(), started.CommandID) }()
		kill := nextFrame(t, l).(*runnerwire.JobKill)
		if kill.Signal == nil || *kill.Signal != runnerwire.SignalTerminate {
			t.Fatal(kill)
		}
		sendFrame(t, l, &runnerwire.JobExit{JobID: job.JobID, Signal: new("SIGTERM"), Files: []runnerwire.JobFileChange{}})
		requirePipe(t, <-aborted)
		_ = nextFrame(t, l).(*runnerwire.JobRelease)
		hint("late", new("arrived after the end"), job.JobID)
		status, err := s.Status(started.CommandID)
		requirePipe(t, err)
		if status.State.Phase != host.Aborted {
			t.Fatal(status.State)
		}
		running, err := d.Host("/work", nil).StartJob(t.Context(), startRequest("attend"))
		requirePipe(t, err)
		nextFrame(t, l)
		hint("i1", new("attending"), running.ID())
		if !reflect.DeepEqual(running.RunningHint(), new("attending")) {
			t.Fatal(running.RunningHint())
		}
		_, err = l.Close(t.Context())
		requirePipe(t, err)
		end, err := running.End(t.Context())
		requirePipe(t, err)
		if end.Status.Kind != host.ProcessLost || running.RunningHint() != nil {
			t.Fatal(end)
		}
	})
}

func TestWatchedCommandIsFollowedAndPageHoldsRunnerOutput(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		d, l, s, p := shellFixture(t, nil)
		started := shellExec(t, s, "build")
		initial := page(t, p)
		if initial.CommandID != started.CommandID || initial.ToolUseID != "call" || initial.State.Phase != host.Running || initial.Tail != "" || initial.Chars != 0 {
			t.Fatal(initial)
		}
		job := nextFrame(t, l).(*runnerwire.JobStart)
		p.Watch(true)
		if !nextFrame(t, l).(*runnerwire.JobFollow).Follow {
			t.Fatal("not followed")
		}
		outputFrame(t, l, job.JobID, 0, []byte("first\n"))
		if page(t, p).Tail != "first\n" {
			t.Fatal("missing first view")
		}
		outputFrame(t, l, job.JobID, 6, bytes.Repeat([]byte{'x'}, runnerwire.JobViewBytes-6))
		page(t, p)
		outputFrame(t, l, job.JobID, runnerwire.JobViewBytes+10, []byte("newest\n"))
		view := page(t, p)
		note := "\n[... 10 bytes of stdout not shown ...]\n"
		if !strings.HasSuffix(view.Tail, "x"+note+"newest\n") || view.Chars != uint64(runnerwire.JobViewBytes+len(note)+7) {
			t.Fatal(view)
		}
		status, err := s.Status(started.CommandID)
		requirePipe(t, err)
		if !strings.HasSuffix(status.Stdout.Tail, "x") || status.Stdout.Bytes != runnerwire.JobViewBytes+17 || status.Unreceived != 17 {
			t.Fatal(status)
		}
		p.Watch(false)
		if nextFrame(t, l).(*runnerwire.JobFollow).Follow {
			t.Fatal("still followed")
		}
		stream := append([]byte("first\n"), bytes.Repeat([]byte{'x'}, runnerwire.JobViewBytes-6)...)
		stream = append(stream, []byte("yyyyyyyyyynewest\nlast\n")...)
		sendFrame(t, l, &runnerwire.JobExit{JobID: job.JobID, ExitCode: new(int32(0)), Output: &runnerwire.OutputLengths{StdoutBytes: uint64(len(stream))}, Files: []runnerwire.JobFileChange{}})
		records := []host.OutputRecord{{Stream: core.StreamKindStdout, Bytes: stream}}
		data, err := remotehost.EncodeOutput(host.WholeOutput{Records: records})
		requirePipe(t, err)
		answerOutput(t, d, l, data)
		end := terminalPage(t, p)
		if end.State.Phase != host.Exited || end.State.ExitCode != 0 || !strings.HasSuffix(end.Tail, "newest\nlast\n") {
			t.Fatal(end)
		}
		status, err = s.Status(started.CommandID)
		requirePipe(t, err)
		if status.Whole == nil || !reflect.DeepEqual(status.Whole.Output.Records, records) {
			t.Fatal(status.Whole)
		}
		if nextFrame(t, l).(*runnerwire.JobRelease).JobID != job.JobID {
			t.Fatal("wrong job release")
		}
		if len(p.Drain()) != 0 {
			t.Fatal("duplicate page publication")
		}
		p.Watch(true)
		shellExec(t, s, "again")
		_ = nextFrame(t, l).(*runnerwire.JobStart)
		if !nextFrame(t, l).(*runnerwire.JobFollow).Follow {
			t.Fatal("watched start not followed")
		}
	})
}

// editPublisher holds publication until the test has inspected the running command.
type editPublisher struct {
	entered, release chan struct{}
	file             core.EditedFile
}

func (p *editPublisher) Retain(ctx context.Context, _ core.CommandID, _ []runnerwire.JobFileChange) ([]core.EditedFile, error) {
	close(p.entered)
	select {
	case <-p.release:
		return []core.EditedFile{p.file}, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}
func (*editPublisher) KeepOutput(context.Context, core.CommandID, host.WholeOutput) error { return nil }

func TestCommandEndsOnceEditsPublishedAndKeepsThem(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		file := core.EditedFile{Path: "/work/file", Kind: core.EditKindModified, Added: 1, Removed: 1, Edits: []core.EditSegment{{Copies: &core.EditCopies{Original: core.BlobRefOf([]byte("before\n")), Modified: core.BlobRefOf([]byte("after\n"))}}}}
		keeper := &editPublisher{entered: make(chan struct{}), release: make(chan struct{}), file: file}
		_, l, s, p := shellFixture(t, func(o *remotehost.EnvironmentOptions) { o.Keeper = keeper })
		started := shellExec(t, s, "echo new > file")
		page(t, p)
		job := nextFrame(t, l).(*runnerwire.JobStart)
		sendFrame(t, l, &runnerwire.JobExit{JobID: job.JobID, ExitCode: new(int32(7)), Files: []runnerwire.JobFileChange{{Path: "/work/file", Kind: commandwire.EditKind("modified"), Added: 1, Removed: 1, Edits: []commandwire.EditCopies{{Original: new("/copies/before"), Modified: new("/copies/after")}}}}, FilesTruncated: true})
		<-keeper.entered
		status, err := s.Status(started.CommandID)
		requirePipe(t, err)
		if status.State.Phase != host.Running {
			t.Fatal("settled before edit publication")
		}
		close(keeper.release)
		terminalPage(t, p)
		for range 2 {
			status, err = s.Status(started.CommandID)
			requirePipe(t, err)
			if status.State.Phase != host.Exited || status.State.ExitCode != 7 || status.Files == nil || !status.Files.Truncated || !reflect.DeepEqual(status.Files.Files, []core.EditedFile{file}) {
				t.Fatal(status)
			}
		}
	})
}

// failingKeeper returns the already retained edits alongside its diagnostic.
type failingKeeper struct{ *editPublisher }

func (p failingKeeper) Retain(ctx context.Context, id core.CommandID, changes []runnerwire.JobFileChange) ([]core.EditedFile, error) {
	files, err := p.editPublisher.Retain(ctx, id, changes)
	if err != nil {
		return nil, err
	}
	return files, errors.New("edit storage unavailable")
}
func (failingKeeper) KeepOutput(context.Context, core.CommandID, host.WholeOutput) error {
	return errors.New("output storage unavailable")
}

func TestKeeperAndReleaseFailuresAreLoggedWithoutChangingCompletion(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var log bytes.Buffer
		previous := slog.Default()
		slog.SetDefault(slog.New(slog.NewTextHandler(&log, &slog.HandlerOptions{Level: slog.LevelDebug})))
		defer slog.SetDefault(previous)
		file := core.EditedFile{Path: "/work/file", Kind: core.EditKindModified, Added: 1, Edits: []core.EditSegment{}}
		publisher := &editPublisher{entered: make(chan struct{}), release: make(chan struct{}), file: file}
		_, l, s, p := shellFixture(t, func(options *remotehost.EnvironmentOptions) { options.Keeper = failingKeeper{publisher} })
		started := shellExec(t, s, "echo new > file")
		page(t, p)
		job := nextFrame(t, l).(*runnerwire.JobStart)
		sendFrame(t, l, &runnerwire.JobExit{JobID: job.JobID, ExitCode: new(int32(7)), Files: []runnerwire.JobFileChange{{Path: "/work/file", Kind: commandwire.EditModified, Added: 1, Edits: []commandwire.EditCopies{}}}})
		<-publisher.entered
		_, err := l.Close(t.Context())
		requirePipe(t, err)
		close(publisher.release)
		terminalPage(t, p)
		status, err := s.Status(started.CommandID)
		requirePipe(t, err)
		if status.State.Phase != host.Exited || status.State.ExitCode != 7 || status.Files == nil || !reflect.DeepEqual(status.Files.Files, []core.EditedFile{file}) || status.Whole == nil {
			t.Fatal(status)
		}
		for _, text := range []string{"an edit's copies were not stored: edit storage unavailable", "a command's output was not stored: output storage unavailable", "job release not sent: runner disconnected"} {
			if !strings.Contains(log.String(), text) {
				t.Fatalf("missing %q in %s", text, log.String())
			}
		}
	})
}
