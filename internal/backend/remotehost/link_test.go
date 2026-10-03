package remotehost_test

import (
	"bytes"
	"errors"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerwire"
)

// linkDevice gives each scenario a joined fake runner and its real backend engine.
func linkDevice(t *testing.T) (*remotehosttest.TestDevice, *remotehosttest.TestLink, *remotehost.Host) {
	t.Helper()
	d := remotehosttest.NewTestDevice(t, remotehosttest.NewCommandPolicy(nil))
	l := d.Connect(nil)
	return d, l, d.Host("/work", nil)
}

// nextFrame observes the next request at the runner boundary.
func nextFrame(t *testing.T, l *remotehosttest.TestLink) runnerwire.Inbound {
	t.Helper()
	frame, err := l.Next(t.Context())
	requirePipe(t, err)
	return frame
}

// sendFrame supplies a validated runner response.
func sendFrame(t *testing.T, l *remotehosttest.TestLink, message runnerwire.Outbound) {
	t.Helper()
	requirePipe(t, l.Send(t.Context(), message))
}

// startRequest supplies the command context required by a shell job.
func startRequest(script string) remotehost.JobStart {
	return remotehost.JobStart{Script: script, CWD: "/work", Env: map[string]string{}, Context: hosttest.CommandContext()}
}

// barrier observes all earlier runner responses without scheduler polling.
func barrier(t *testing.T, l *remotehosttest.TestLink) {
	t.Helper()
	done := make(chan error, 1)
	go func() { done <- l.Link().Sync(t.Context()) }()
	request, ok := nextFrame(t, l).(*runnerwire.Sync)
	if !ok {
		t.Fatal("expected synchronization request")
	}
	sendFrame(t, l, &runnerwire.SyncDone{ID: request.ID})
	requirePipe(t, <-done)
}

// Cost: in-process frames only; timer scenarios use virtual time.
func TestFSReplyIsReadAsTheRequestedOperation(t *testing.T) {
	_, l, h := linkDevice(t)
	done := make(chan error, 1)
	go func() {
		_, err := h.FS().Stat(t.Context(), "/work/file")
		done <- err
	}()
	request := nextFrame(t, l).(*runnerwire.FSStat)
	sendFrame(t, l, &runnerwire.FSOK{ID: request.ID, Result: &runnerwire.FSReadlinkResult{Value: "/work/elsewhere"}})
	var failure *host.Error
	if !errors.As(<-done, &failure) || failure.Kind != host.Protocol {
		t.Fatalf("wrong failure: %v", failure)
	}
}

func TestAdmissionHoldsCallsAndProcessLifetimesAndRefusalSendsNothing(t *testing.T) {
	d, l, _ := linkDevice(t)
	gate := gates.NewActivity(nil)
	var blocked atomic.Bool
	h := d.Host("/work", func() (*gates.Lease, error) {
		if blocked.Load() {
			return nil, &host.Error{Kind: host.Unavailable, Message: "machine transition"}
		}
		return gate.TryEnter(gates.Demand), nil
	})
	done := make(chan error, 1)
	go func() {
		exists, err := h.FS().Exists(t.Context(), "/work/file")
		if err == nil && !exists {
			err = errors.New("file missing")
		}
		done <- err
	}()
	request := nextFrame(t, l).(*runnerwire.FSExists)
	if gate.State().Demand != 1 {
		t.Fatal("call admission missing")
	}
	sendFrame(t, l, &runnerwire.FSOK{ID: request.ID, Result: &runnerwire.FSExistsResult{Value: true}})
	requirePipe(t, <-done)
	if gate.State().Demand != 0 {
		t.Fatal("call admission leaked")
	}
	process, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "sleep"})
	requirePipe(t, err)
	job, err := h.StartJob(t.Context(), startRequest("sleep 10"))
	requirePipe(t, err)
	nextFrame(t, l)
	nextFrame(t, l)
	if gate.State().Demand != 2 {
		t.Fatal("work admission missing")
	}
	blocked.Store(true)
	_, err = h.FS().ReadFile(t.Context(), "/work/next")
	var failure *host.Error
	if !errors.As(err, &failure) || failure.Message != "machine transition" {
		t.Fatal(err)
	}
	if _, err = h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "true"}); err == nil {
		t.Fatal("spawn admitted")
	}
	if _, err = h.StartJob(t.Context(), startRequest("true")); err == nil {
		t.Fatal("job admitted")
	}
	barrier(t, l)
	if _, ok, err := l.TryNext(); ok || err != nil {
		t.Fatal("refusal emitted a frame", err)
	}
	closed, err := l.Close(t.Context())
	requirePipe(t, err)
	if closed.Kind != remotehost.LinkClosed || closed.Reason != "runner disconnected" {
		t.Fatal(closed)
	}
	end, err := process.Wait(t.Context())
	requirePipe(t, err)
	jobEnd, err := job.End(t.Context())
	requirePipe(t, err)
	if end.Kind != host.ProcessLost || jobEnd.Status.Kind != host.ProcessLost || gate.State().Demand != 0 {
		t.Fatal("disconnect failed to release work")
	}
}

func TestClosingRunningProcessKillsItAndEndedOneIsLeftAlone(t *testing.T) {
	_, l, h := linkDevice(t)
	running, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "sleep"})
	requirePipe(t, err)
	first := nextFrame(t, l).(*runnerwire.Spawn)
	ended, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "true"})
	requirePipe(t, err)
	second := nextFrame(t, l).(*runnerwire.Spawn)
	sendFrame(t, l, &runnerwire.SpawnExit{SpawnID: second.SpawnID, ExitCode: new(int32(0))})
	result, err := ended.Wait(t.Context())
	requirePipe(t, err)
	if result.Kind != host.ProcessExited || result.ExitCode != 0 {
		t.Fatal(result)
	}
	requirePipe(t, ended.Control.Close(t.Context()))
	requirePipe(t, running.Control.Close(t.Context()))
	killed := nextFrame(t, l).(*runnerwire.SpawnKill)
	if killed.SpawnID != first.SpawnID || killed.Signal == nil || *killed.Signal != runnerwire.SignalKill {
		t.Fatal(killed)
	}
	barrier(t, l)
}

func TestRetainedProcessHoldsNoAdmissionAndIsNotCounted(t *testing.T) {
	d, l, _ := linkDevice(t)
	gate := gates.NewActivity(nil)
	h := d.Host("/work", func() (*gates.Lease, error) { return gate.TryEnter(gates.Demand), nil })
	retained, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "provider", Retained: true})
	requirePipe(t, err)
	nextFrame(t, l)
	if gate.State().Demand != 0 || l.Link().RunningJobs() != 0 {
		t.Fatal("retained process counted as demand")
	}
	work, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "sleep"})
	requirePipe(t, err)
	nextFrame(t, l)
	if gate.State().Demand != 1 || l.Link().RunningJobs() != 1 {
		t.Fatal("ordinary process not counted")
	}
	_, err = l.Close(t.Context())
	requirePipe(t, err)
	_, err = retained.Wait(t.Context())
	requirePipe(t, err)
	_, err = work.Wait(t.Context())
	requirePipe(t, err)
	if gate.State().Demand != 0 {
		t.Fatal("admission leaked")
	}
}

func TestStdinTravelsInBoundedFramesInOrderBeforeEnd(t *testing.T) {
	_, l, h := linkDevice(t)
	p, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "cat"})
	requirePipe(t, err)
	j, err := h.StartJob(t.Context(), startRequest("cat"))
	requirePipe(t, err)
	nextFrame(t, l)
	nextFrame(t, l)
	for _, size := range []int{0, runnerwire.StdinChunkBytes, runnerwire.StdinChunkBytes + 1} {
		data := make([]byte, size)
		for i := range data {
			data[i] = byte(i % 256)
		}
		requirePipe(t, p.Control.WriteStdin(t.Context(), data))
		requirePipe(t, j.WriteStdin(t.Context(), data))
		var spawned, jobbed []byte
		for range 2 * ((size + runnerwire.StdinChunkBytes - 1) / runnerwire.StdinChunkBytes) {
			frame := nextFrame(t, l)
			if spawnedFrame, ok := frame.(*runnerwire.SpawnStdin); ok {
				if len(spawnedFrame.Bytes) > runnerwire.StdinChunkBytes {
					t.Fatal("oversize stdin")
				}
				spawned = append(spawned, spawnedFrame.Bytes...)
			} else if jobFrame, ok := frame.(*runnerwire.JobStdin); ok {
				if len(jobFrame.Bytes) > runnerwire.StdinChunkBytes {
					t.Fatal("oversize stdin")
				}
				jobbed = append(jobbed, jobFrame.Bytes...)
			} else {
				t.Fatalf("unexpected %T", frame)
			}
		}
		if !bytes.Equal(spawned, data) || !bytes.Equal(jobbed, data) {
			t.Fatal("stdin changed")
		}
		barrier(t, l)
	}
	requirePipe(t, p.Control.CloseStdin(t.Context()))
	requirePipe(t, j.CloseStdin(t.Context()))
	if _, ok := nextFrame(t, l).(*runnerwire.SpawnStdinEnd); !ok {
		t.Fatal("missing spawn EOF")
	}
	if _, ok := nextFrame(t, l).(*runnerwire.JobStdinEnd); !ok {
		t.Fatal("missing job EOF")
	}
	barrier(t, l)
}

func TestLostConnectionFailsItsWorkAndNextServesSameHost(t *testing.T) {
	d := remotehosttest.NewTestDevice(t, remotehosttest.NewCommandPolicy(nil))
	h := d.Host("/work", nil)
	if h.Identity().Hostname != "" {
		t.Fatal("invented identity")
	}
	l := d.Connect(nil)
	identity := h.Identity()
	if identity.UID != 501 || identity.Hostname != "test" {
		t.Fatal(identity)
	}
	done := make(chan error, 1)
	go func() {
		_, err := h.FS().ReadFile(t.Context(), "/work/x")
		done <- err
	}()
	nextFrame(t, l)
	p, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "sleep"})
	requirePipe(t, err)
	j, err := h.StartJob(t.Context(), startRequest("sleep 30"))
	requirePipe(t, err)
	nextFrame(t, l)
	nextFrame(t, l)
	closed, err := l.Close(t.Context())
	requirePipe(t, err)
	if closed.Kind != remotehost.LinkClosed || closed.Reason != "runner disconnected" {
		t.Fatal(closed)
	}
	var failure *host.Error
	if !errors.As(<-done, &failure) || failure.Kind != host.Offline || failure.Message != "runner disconnected" {
		t.Fatal(failure)
	}
	pEnd, err := p.Wait(t.Context())
	requirePipe(t, err)
	jEnd, err := j.End(t.Context())
	requirePipe(t, err)
	if pEnd.Kind != host.ProcessLost || pEnd.Reason != "runner disconnected" || jEnd.Status.Kind != host.ProcessLost || jEnd.Status.Reason != "runner disconnected" || h.Identity() != identity {
		t.Fatal("lost work or identity")
	}
	p, err = h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "echo"})
	requirePipe(t, err)
	pEnd, err = p.Wait(t.Context())
	requirePipe(t, err)
	j, err = h.StartJob(t.Context(), startRequest("true"))
	requirePipe(t, err)
	jEnd, err = j.End(t.Context())
	requirePipe(t, err)
	if pEnd.Kind != host.ProcessLost || jEnd.Status.Kind != host.ProcessLost {
		t.Fatal("offline work stayed running")
	}
	_, err = h.FS().Exists(t.Context(), "/work")
	if !errors.As(err, &failure) || failure.Kind != host.Offline {
		t.Fatal(err)
	}
	l = d.Connect(nil)
	go func() {
		exists, err := h.FS().Exists(t.Context(), "/work")
		if err == nil && !exists {
			err = errors.New("file missing")
		}
		done <- err
	}()
	request := nextFrame(t, l).(*runnerwire.FSExists)
	sendFrame(t, l, &runnerwire.FSOK{ID: request.ID, Result: &runnerwire.FSExistsResult{Value: true}})
	requirePipe(t, <-done)
}

func TestMalformedFrameEndsConnection(t *testing.T) {
	_, l, _ := linkDevice(t)
	requirePipe(t, l.SendFrame(t.Context(), []byte(`{"type":"unknown"}`)))
	end, err := l.Ended(t.Context())
	requirePipe(t, err)
	if end.Kind != remotehost.LinkRefused {
		t.Fatal(end)
	}
}

func TestUnansweredPingEndsUnlessLivenessPaused(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		d := remotehosttest.NewTestDevice(t, remotehosttest.NewCommandPolicy(nil))
		l := d.Connect(new(time.Second))
		time.Sleep(time.Second)
		if _, ok := nextFrame(t, l).(*runnerwire.Ping); !ok {
			t.Fatal("missing initial ping")
		}
		sendFrame(t, l, &runnerwire.Pong{Jobs: 2})
		barrier(t, l)
		if l.Link().RunningJobs() != 2 {
			t.Fatal("pong job count lost")
		}
		time.Sleep(time.Second)
		if _, ok := nextFrame(t, l).(*runnerwire.Ping); !ok {
			t.Fatal("missing second ping")
		}
		l.Link().PauseLiveness()
		time.Sleep(4 * time.Second)
		if l.Link().IsClosed() {
			t.Fatal("paused connection expired")
		}
		if _, ok, err := l.TryNext(); ok || err != nil {
			t.Fatal("paused liveness emitted a frame", err)
		}
		l.Link().ResumeLiveness()
		time.Sleep(time.Second)
		if _, ok := nextFrame(t, l).(*runnerwire.Ping); !ok {
			t.Fatal("missing ping")
		}
		time.Sleep(time.Second)
		end, err := l.Ended(t.Context())
		requirePipe(t, err)
		if end.Kind != remotehost.LinkDisconnected || end.Reason != "liveness: ping unanswered" {
			t.Fatal(end)
		}
	})
}
