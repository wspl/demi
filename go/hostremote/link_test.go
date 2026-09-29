package hostremote_test

import (
	"errors"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/hostremote"
	"github.com/wspl/demi/go/hostremote/hostremotetest"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
	"github.com/wspl/demi/go/shell/shelltest"
)

func next(t *testing.T, link *hostremotetest.TestLink) runnerproto.Inbound {
	t.Helper()
	v, err := link.Next(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	return v
}
func send(t *testing.T, link *hostremotetest.TestLink, v runnerproto.Outbound) {
	t.Helper()
	if err := link.Send(t.Context(), v); err != nil {
		t.Fatal(err)
	}
}

// Cost: one in-process connection. Replies are deliberately reversed and one
// has the wrong operation, so routing is observed through concurrent Host calls.
func TestLinkRoutesRepliesAndReleasesDisconnectedWork(t *testing.T) {
	device := hostremotetest.NewTestDevice(t, nil)
	host := device.Host("/work", nil)
	link := device.Connect(0)
	exists := make(chan error, 1)
	stat := make(chan error, 1)
	go func() {
		yes, err := host.Exists(t.Context(), "a")
		if err == nil && !yes {
			err = errors.New("exists false")
		}
		exists <- err
	}()
	go func() {
		_, err := host.Stat(t.Context(), "b")
		stat <- err
	}()
	var existsID, statID string
	for range 2 {
		switch m := next(t, link).(type) {
		case runnerproto.InboundFSExists:
			existsID = m.ID
		case runnerproto.InboundFSStat:
			statID = m.ID
		default:
			t.Fatalf("request %T", m)
		}
	}
	send(t, link, runnerproto.OutboundFSOk{ID: statID, Result: runnerproto.FSResultExists{Value: true}})
	send(t, link, runnerproto.OutboundFSOk{ID: existsID, Result: runnerproto.FSResultExists{Value: true}})
	if err := <-exists; err != nil {
		t.Fatal(err)
	}
	if err := <-stat; err == nil || err.Error() != `the runner answered Fs("exists") to a Fs("stat") request` {
		t.Fatal(err)
	}
	process, err := host.Spawn(t.Context(), shell.SpawnRequest{Command: "cat"})
	if err != nil {
		t.Fatal(err)
	}
	spawn := next(t, link).(runnerproto.InboundSpawn)
	payload := strings.Repeat("x", 65537)
	if err := process.Control.WriteStdin(t.Context(), []byte(payload)); err != nil {
		t.Fatal(err)
	}
	if err := process.Control.CloseStdin(t.Context()); err != nil {
		t.Fatal(err)
	}
	first := next(t, link).(runnerproto.InboundSpawnStdin)
	second := next(t, link).(runnerproto.InboundSpawnStdin)
	if len(first.Bytes) != 65536 || len(second.Bytes) != 1 || first.SpawnID != spawn.SpawnID {
		t.Fatal("stdin framing")
	}
	if _, ok := next(t, link).(runnerproto.InboundSpawnStdinEnd); !ok {
		t.Fatal("stdin end out of order")
	}
	go func() {
		_, err := host.Exists(t.Context(), "pending")
		exists <- err
	}()
	next(t, link)
	link.Close()
	if err := <-exists; err == nil {
		t.Fatal("pending request survived loss")
	}
	end, err := process.Wait(t.Context())
	if err != nil || end.Kind != shell.ProcessLost {
		t.Fatal(end, err)
	}
	reconnected := device.Connect(0)
	go func() {
		yes, err := host.Exists(t.Context(), "again")
		if err == nil && !yes {
			err = errors.New("false")
		}
		exists <- err
	}()
	request := next(t, reconnected).(runnerproto.InboundFSExists)
	send(t, reconnected, runnerproto.OutboundFSOk{ID: request.ID, Result: runnerproto.FSResultExists{Value: true}})
	if err := <-exists; err != nil {
		t.Fatal(err)
	}
}

// Cost: fake time only. Read the first ping before advancing its reply deadline.
func TestLinkLivenessPauseAndProtocolRefusal(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		device := hostremotetest.NewTestDevice(t, nil)
		link := device.Connect(time.Second)
		synctest.Wait()
		time.Sleep(time.Second)
		if _, ok := next(t, link).(runnerproto.InboundPing); !ok {
			t.Fatal("no ping")
		}
		link.Link.PauseLiveness()
		time.Sleep(3 * time.Second)
		if link.Link.IsClosed() {
			t.Fatal("paused liveness expired")
		}
		link.Link.ResumeLiveness()
		time.Sleep(time.Second)
		next(t, link)
		time.Sleep(time.Second)
		end, err := link.Ended(t.Context())
		if err != nil || end.Reason != "liveness: ping unanswered" {
			t.Fatal(end, err)
		}
		bad := device.Connect(0)
		if err := bad.SendFrame(t.Context(), []byte{0xc1}); err != nil {
			t.Fatal(err)
		}
		end, err = bad.Ended(t.Context())
		if err != nil || end.Kind != "refused" {
			t.Fatal(end, err)
		}
	})
}
func jobStart() hostremote.JobStart {
	return hostremote.JobStart{Script: "echo ready", Cwd: "/work", Env: map[string]string{}, Context: shelltest.CommandContext()}
}

// Cost: one fake connection. Admission is inspected through the shard executor;
// disconnect joins lease releases before the final count is read.
func TestAdmissionHoldsWorkAndRefusesWithoutSending(t *testing.T) {
	device := hostremotetest.NewTestDevice(t, nil)
	link := device.Connect(0)
	active, blocked := 0, false
	host := device.Host("/work", func() (func(), error) {
		if blocked {
			return nil, errors.New("machine transition")
		}
		active++
		return func() { active-- }, nil
	})
	process, err := host.Spawn(t.Context(), shell.SpawnRequest{Command: "cat"})
	if err != nil {
		t.Fatal(err)
	}
	next(t, link)
	job, err := host.StartJob(t.Context(), jobStart())
	if err != nil {
		t.Fatal(err)
	}
	next(t, link)
	retained, err := host.Spawn(t.Context(), shell.SpawnRequest{Command: "retained", Retained: true})
	if err != nil {
		t.Fatal(err)
	}
	next(t, link)
	device.Execute(t.Context(), func() {
		if active != 2 {
			t.Errorf("leases %d", active)
		}
		blocked = true
	})
	if link.Link.RunningJobs() != 2 {
		t.Fatal("retained process counted")
	}
	if _, err := host.ReadFile(t.Context(), "no"); err == nil || err.Error() != "machine transition" {
		t.Fatal(err)
	}
	if _, err := host.Spawn(t.Context(), shell.SpawnRequest{Command: "no"}); err == nil {
		t.Fatal("spawn admitted")
	}
	if _, err := host.StartJob(t.Context(), jobStart()); err == nil {
		t.Fatal("job admitted")
	}
	if _, available, err := link.TryNext(); err != nil || available {
		t.Fatal("refusal sent frame")
	}
	process.Close()
	if _, ok := next(t, link).(runnerproto.InboundSpawnKill); !ok {
		t.Fatal("drop did not kill")
	}
	link.Close()
	if end, err := job.End(t.Context()); err != nil || end.Status.Kind != shell.ProcessLost {
		t.Fatal(end, err)
	}
	if end, err := retained.Wait(t.Context()); err != nil || end.Kind != shell.ProcessLost {
		t.Fatal(end, err)
	}
	device.Execute(t.Context(), func() {
		if active != 0 {
			t.Errorf("leases leaked: %d", active)
		}
	})
}
