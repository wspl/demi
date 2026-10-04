package remotehost_test

import (
	"context"
	"errors"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/runnerproto"
)

// logMatching reads across the log writer's publication until the scenario's event appears.
// The runner publishes no event for a written log line, so each retry is a real
// read that waits for the runner's reply.
func logMatching(
	t *testing.T,
	h *remotehost.Host,
	after *uint64,
	source *string,
	match func(remotehost.LogPage) bool,
) remotehost.LogPage {
	t.Helper()
	for {
		page, err := h.ReadLog(t.Context(), after, 1000, source)
		requirePipe(t, err)
		if match(page) {
			return page
		}
	}
}

func logCount(page remotehost.LogPage, text string) int {
	count := 0
	for _, line := range page.Lines {
		if line.Text == text {
			count++
		}
	}
	return count
}

func TestRunnerLogNewestLinesThenLaterLines(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	h := f.Host()
	page := logMatching(
		t,
		h,
		nil,
		new("runner"),
		func(page remotehost.LogPage) bool {
			return logCount(page, "online") > 0
		},
	)
	started := false
	for _, line := range page.Lines {
		if line.Source != "runner" {
			t.Fatal(line)
		}
		if version, ok := strings.CutPrefix(line.Text, "runner "); ok {
			version, ok = strings.CutSuffix(version, " started")
			started = started || (ok && version != "" && !strings.Contains(version, " "))
		}
	}
	if !started || page.Next == 0 {
		t.Fatal(page)
	}
	after, err := h.ReadLog(t.Context(), &page.Next, 50, nil)
	requirePipe(t, err)
	if len(after.Lines) != 0 || after.Next != page.Next {
		t.Fatal(after)
	}
	one, err := h.ReadLog(t.Context(), new(uint64(0)), 1, nil)
	requirePipe(t, err)
	if len(one.Lines) != 1 || one.Next >= page.Next {
		t.Fatal(one)
	}
	other, err := h.ReadLog(t.Context(), new(uint64(0)), 50, new("service:none"))
	requirePipe(t, err)
	if len(other.Lines) != 0 || other.Next != page.Next {
		t.Fatal(other)
	}
}

func TestRunnerWorkingTreeChangesAndLastCommit(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	h := f.Host()
	home := f.Home()
	git := func(args ...string) {
		t.Helper()
		command := exec.CommandContext(t.Context(), "git", args...)
		command.Dir = home
		command.Env = append(
			os.Environ(),
			"GIT_AUTHOR_NAME=Test",
			"GIT_AUTHOR_EMAIL=test@example.com",
			"GIT_COMMITTER_NAME=Test",
			"GIT_COMMITTER_EMAIL=test@example.com",
		)
		output, err := command.CombinedOutput()
		if err != nil {
			t.Fatalf("git %v: %v: %s", args, err, output)
		}
	}
	outside, err := h.GitChanges(t.Context(), home)
	requirePipe(t, err)
	if outside.Repository || outside.Head != nil || len(outside.Files) != 0 || outside.Truncated {
		t.Fatal(outside)
	}
	git("init", "-q", "-b", "main")
	requirePipe(t, os.WriteFile(filepath.Join(home, "a.txt"), []byte("1\n2\n"), 0o600))
	git("add", ".")
	git("commit", "-q", "-m", "first")
	requirePipe(t, os.WriteFile(filepath.Join(home, "a.txt"), []byte("1\n2\n3\n"), 0o600))
	requirePipe(t, os.WriteFile(filepath.Join(home, "b.txt"), []byte("new\n"), 0o600))
	changes, err := h.GitChanges(t.Context(), home)
	requirePipe(t, err)
	if !changes.Repository || changes.Head == nil || len(*changes.Head) != 40 {
		t.Fatal(changes)
	}
	for _, digit := range *changes.Head {
		if !strings.ContainsRune("0123456789abcdefABCDEF", digit) {
			t.Fatal(*changes.Head)
		}
	}
	want := []runnerproto.GitChange{
		{Path: "a.txt", Status: " M", Kind: runnerproto.ChangeKindModified, Added: 1},
		{Path: "b.txt", Status: "??", Kind: runnerproto.ChangeKindAdded, Added: 1},
	}
	if !reflect.DeepEqual(changes.Files, want) {
		t.Fatal(changes.Files)
	}
	original, err := h.GitShow(t.Context(), home, "a.txt")
	requirePipe(t, err)
	if string(original) != "1\n2\n" {
		t.Fatal(string(original))
	}
	_, err = h.GitShow(t.Context(), home, "b.txt")
	requireHostCode(t, err, "ENOENT")
}

// socketPeer owns one loopback peer and joins its IO on test cleanup, including failure.
func socketPeer(t *testing.T, serve func(*net.TCPConn) error) (uint16, <-chan struct{}, <-chan error) {
	t.Helper()
	listener, err := net.ListenTCP("tcp", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1)})
	requirePipe(t, err)
	ctx, cancel := context.WithCancel(t.Context())
	accepted := make(chan struct{})
	done := make(chan struct{})
	result := make(chan error, 1)
	go func() {
		defer close(done)
		listenerCanceled := make(chan struct{})
		stopListen := context.AfterFunc(ctx, func() {
			defer close(listenerCanceled)
			if err := listener.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
				t.Error(err)
			}
		})
		conn, err := listener.AcceptTCP()
		if !stopListen() {
			<-listenerCanceled
		}
		if err != nil {
			result <- err
			return
		}
		close(accepted)
		canceled := make(chan struct{})
		stop := context.AfterFunc(ctx, func() {
			defer close(canceled)
			if err := conn.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
				t.Error(err)
			}
		})
		err = serve(conn)
		if !stop() {
			<-canceled
		}
		closeErr := conn.Close()
		if errors.Is(closeErr, net.ErrClosed) {
			closeErr = nil
		}
		result <- errors.Join(err, closeErr)
	}()
	t.Cleanup(func() {
		cancel()
		if err := listener.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
			t.Error(err)
		}
		<-done
	})
	return uint16(listener.Addr().(*net.TCPAddr).Port), accepted, result
}

func TestRunnerNetworkSocketThroughTwoPipes(t *testing.T) {
	tap := newWireTap()
	f := runnerFixture(t, remotehosttest.FixtureOptions{Tap: tap.input})
	h := f.Host()
	echoPort, _, echoDone := socketPeer(t, func(conn *net.TCPConn) error {
		_, err := io.Copy(conn, conn)
		return errors.Join(err, conn.CloseWrite())
	})
	input := f.Pipes().ToDevice(remotehosttest.TestDeviceID)
	output := f.Pipes().FromDevice(remotehosttest.TestDeviceID)
	writer, err := input.Writer()
	requirePipe(t, err)
	reader, err := output.Reader()
	requirePipe(t, err)
	requirePipe(t, h.OpenNet(t.Context(), "127.0.0.1", echoPort, input.WireRef(), output.WireRef()))
	payload := patternedBytes(1024 * 1024)
	fed := make(chan error, 1)
	go func() {
		err := writer.Write(t.Context(), payload)
		if err == nil {
			writer.End()
		} else {
			writer.Fail(err.Error())
		}
		fed <- err
	}()
	data, err := collectPipe(t.Context(), reader)
	requirePipe(t, err)
	requirePipe(t, reader.Close(t.Context()))
	requirePipe(t, <-fed)
	requirePipe(t, <-echoDone)
	if !reflect.DeepEqual(data, payload) {
		t.Fatal("socket echo changed")
	}
	for _, id := range []string{input.ID(), output.ID()} {
		done := tap.pipeDone(t, id)
		if !done.Ok || done.Error != nil {
			t.Fatal(done)
		}
	}
	vacant, err := net.ListenTCP("tcp", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1)})
	requirePipe(t, err)
	port := uint16(vacant.Addr().(*net.TCPAddr).Port)
	requirePipe(t, vacant.Close())
	err = h.OpenNet(
		t.Context(),
		"127.0.0.1",
		port,
		f.Pipes().ToDevice(remotehosttest.TestDeviceID).WireRef(),
		f.Pipes().FromDevice(remotehosttest.TestDeviceID).WireRef(),
	)
	requireHostCode(t, err, "refused")
	resetPort, _, resetDone := socketPeer(t, func(conn *net.TCPConn) error {
		var first [1]byte
		_, err := io.ReadFull(conn, first[:])
		return errors.Join(err, conn.SetLinger(0))
	})
	flow := f.Pipes().ToDevice(remotehosttest.TestDeviceID)
	reaped := f.Pipes().FromDevice(remotehosttest.TestDeviceID)
	flowWriter, err := flow.Writer()
	requirePipe(t, err)
	reapedReader, err := reaped.Reader()
	requirePipe(t, err)
	requirePipe(t, h.OpenNet(t.Context(), "127.0.0.1", resetPort, flow.WireRef(), reaped.WireRef()))
	flowing := make(chan error, 1)
	go func() {
		for {
			if err := flowWriter.Write(t.Context(), make([]byte, 65536)); err != nil {
				flowWriter.Fail(err.Error())
				flowing <- err
				return
			}
		}
	}()
	failed := tap.pipeDone(t, reaped.ID())
	if failed.Ok || failed.Error == nil {
		t.Fatal(failed)
	}
	if _, err := collectPipe(t.Context(), reapedReader); err == nil {
		t.Fatal("reset ended output cleanly")
	}
	requirePipe(t, reapedReader.Close(t.Context()))
	requirePipe(t, <-resetDone)
	if err := <-flowing; err == nil {
		t.Fatal("reset did not stop input")
	}
	speakerPort, _, speakerDone := socketPeer(t, func(conn *net.TCPConn) error {
		if _, err := io.WriteString(conn, "the peer speaks and then holds the connection open\n"); err != nil {
			return err
		}
		_, err := io.Copy(io.Discard, conn)
		return err
	})
	held := f.Pipes().ToDevice(remotehosttest.TestDeviceID)
	spoken := f.Pipes().FromDevice(remotehosttest.TestDeviceID)
	heldWriter, err := held.Writer()
	requirePipe(t, err)
	spokenReader, err := spoken.Reader()
	requirePipe(t, err)
	requirePipe(t, h.OpenNet(t.Context(), "127.0.0.1", speakerPort, held.WireRef(), spoken.WireRef()))
	buf := make([]byte, 65536)
	n, err := spokenReader.Read(t.Context(), buf)
	requirePipe(t, err)
	if !strings.HasPrefix(string(buf[:n]), "the peer speaks") {
		t.Fatal(string(buf[:n]))
	}
	spokenReader.Fail("the visitor's connection ended")
	requirePipe(t, <-speakerDone)
	spokenDone := tap.pipeDone(t, spoken.ID())
	if spokenDone.Ok || spokenDone.Error == nil || tap.pipeDone(t, held.ID()).Ok {
		t.Fatal("failed stream ended cleanly")
	}
	heldWriter.Fail("page closed")
	requirePipe(t, spokenReader.Close(t.Context()))
	quietPort, connected, quietDone := socketPeer(t, func(conn *net.TCPConn) error {
		_, err := io.Copy(io.Discard, conn)
		return err
	})
	silent := f.Pipes().ToDevice(remotehosttest.TestDeviceID)
	unheard := f.Pipes().FromDevice(remotehosttest.TestDeviceID)
	silentWriter, err := silent.Writer()
	requirePipe(t, err)
	unheardReader, err := unheard.Reader()
	requirePipe(t, err)
	requirePipe(t, h.OpenNet(t.Context(), "127.0.0.1", quietPort, silent.WireRef(), unheard.WireRef()))
	<-connected
	link, err := f.Link(t.Context())
	requirePipe(t, err)
	link.Disconnect("the backend went away")
	requirePipe(t, <-quietDone)
	silentWriter.Fail("connection ended")
	requirePipe(t, unheardReader.Close(t.Context()))
}
