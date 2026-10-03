package host

import (
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// pipeService connects the Host's actual pipe client to a scripted backend.
func pipeService(t *testing.T, root string, handler http.HandlerFunc) (*Service, chan []byte) {
	t.Helper()
	server := httptest.NewServer(handler)
	t.Cleanup(server.Close)
	backend, err := runnerwire.ParseBackendURL(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	token, err := runnerwire.ParseDeviceToken(strings.Repeat("a", 64))
	if err != nil {
		t.Fatal(err)
	}
	pipes, err := process.NewPipeClient(backend, func() (runnerwire.DeviceToken, bool) { return token, true })
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := pipes.Close(); err != nil {
			t.Error(err)
		}
	})
	out := make(chan []byte, 32)
	s := New(t.Context(), root, pipes, out)
	t.Cleanup(func() {
		if err := s.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return s, out
}

func pipeDone(t *testing.T, out <-chan []byte, id string, failed bool) {
	t.Helper()
	frame, ok := receiveFrame(t, out).(*runnerwire.PipeDone)
	if !ok || frame.PipeID != id || (frame.Error != nil) != failed {
		t.Fatalf("pipe %s: %#v", id, frame)
	}
}

func TestFilePipesRangesPublicationAndFailures(t *testing.T) {
	root := t.TempDir()
	payload := string([]byte{0, 255, 128, 10})
	uploaded := make(chan string, 4)
	s, out := pipeService(t, root, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/input":
			if _, err := io.WriteString(w, payload); err != nil {
				t.Error(err)
			}
		case "/broken":
			w.Header().Set("Content-Length", "20")
			if _, err := io.WriteString(w, "short"); err != nil {
				t.Error(err)
			}
		case "/output":
			data, err := io.ReadAll(r.Body)
			if err != nil {
				t.Error(err)
			}
			uploaded <- string(data)
		default:
			http.NotFound(w, r)
		}
	})
	yes := true
	if err := s.WriteFile(
		t.Context(),
		runnerwire.FSWriteFile{
			ID:            "write",
			Path:          "nested/a",
			CreateParents: &yes,
			Input:         runnerwire.PipeRef{ID: "in", URL: "/input"},
		},
	); err != nil {
		t.Fatal(err)
	}
	pipeDone(t, out, "in", false)
	fsResult(t, out, nil)
	data, err := os.ReadFile(filepath.Join(root, "nested/a"))
	if err != nil || string(data) != payload {
		t.Fatalf("%v %v", data, err)
	}
	offset, length := uint64(1), uint64(2)
	if err := s.ReadFile(
		t.Context(),
		runnerwire.FSReadFile{
			ID:     "read",
			Path:   "nested/a",
			Offset: &offset,
			Length: &length,
			Output: runnerwire.PipeRef{ID: "out", URL: "/output"},
		},
	); err != nil {
		t.Fatal(err)
	}
	fsResult(t, out, nil)
	pipeDone(t, out, "out", false)
	if got := <-uploaded; got != payload[1:3] {
		t.Fatalf("%q", got)
	}
	if err := s.WriteFile(
		t.Context(),
		runnerwire.FSWriteFile{ID: "broken", Path: "nested/a", Input: runnerwire.PipeRef{ID: "in", URL: "/broken"}},
	); err != nil {
		t.Fatal(err)
	}
	pipeDone(t, out, "in", true)
	if frame := receiveFrame(t, out); frame == nil {
		t.Fatal("missing error")
	} else if _, ok := frame.(*runnerwire.FSError); !ok {
		t.Fatal(frame)
	}
	data, err = os.ReadFile(filepath.Join(root, "nested/a"))
	if err != nil || string(data) != payload {
		t.Fatalf("failed write replaced destination: %q %v", data, err)
	}
	entries, err := os.ReadDir(filepath.Join(root, "nested"))
	if err != nil || len(entries) != 1 {
		t.Fatalf("temporary file leak: %v %v", entries, err)
	}
	if err := s.ReadFile(
		t.Context(),
		runnerwire.FSReadFile{ID: "missing", Path: "missing", Output: runnerwire.PipeRef{ID: "out", URL: "/output"}},
	); err != nil {
		t.Fatal(err)
	}
	frame, ok := receiveFrame(t, out).(*runnerwire.FSError)
	if !ok || frame.Code == nil || *frame.Code != "ENOENT" {
		t.Fatal(frame)
	}
	pipeDone(t, out, "out", true)
}

func TestGitShowPipesAndLimits(t *testing.T) {
	root := repositoryFixture(t)
	writeFixture(t, root, "large", strings.Repeat("x", MaxBlobBytes+1))
	commitFixture(t, root)
	uploaded := make(chan string, 1)
	s, out := pipeService(t, root, func(_ http.ResponseWriter, r *http.Request) {
		data, err := io.ReadAll(r.Body)
		if err != nil {
			t.Error(err)
		}
		uploaded <- string(data)
	})
	if err := s.GitShow(
		t.Context(),
		runnerwire.GitShow{ID: "show", Root: root, Path: "a.txt", Output: runnerwire.PipeRef{ID: "out", URL: "/out"}},
	); err != nil {
		t.Fatal(err)
	}
	if _, ok := receiveFrame(t, out).(*runnerwire.GitOK); !ok {
		t.Fatal("expected git_ok")
	}
	pipeDone(t, out, "out", false)
	if got := <-uploaded; got != "1\n2\n3\n" {
		t.Fatal(got)
	}
	if err := s.GitShow(
		t.Context(),
		runnerwire.GitShow{ID: "large", Root: root, Path: "large", Output: runnerwire.PipeRef{ID: "out", URL: "/out"}},
	); err != nil {
		t.Fatal(err)
	}
	frame, ok := receiveFrame(t, out).(*runnerwire.GitError)
	if !ok || frame.Code != "too_large" {
		t.Fatal(frame)
	}
	pipeDone(t, out, "out", true)
}

func TestNetworkOpenFailureReportsBothUnusedPipes(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	_, portText, err := net.SplitHostPort(listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
	port, err := strconv.ParseUint(portText, 10, 16)
	if err != nil {
		t.Fatal(err)
	}
	s, out := testService(t, t.TempDir(), MaxFiles)
	if err := s.NetOpen(
		t.Context(),
		runnerwire.NetOpen{
			StreamID: "refused",
			Host:     "127.0.0.1",
			Port:     uint16(port),
			Input:    runnerwire.PipeRef{ID: "in"},
			Output:   runnerwire.PipeRef{ID: "out"},
		},
	); err != nil {
		t.Fatal(err)
	}
	if len(out) != 3 {
		t.Fatalf("open failure delivered %d frames; want net_error and both pipe ends", len(out))
	}
	frame, ok := receiveFrame(t, out).(*runnerwire.NetError)
	if !ok || frame.Code != runnerwire.NetErrorCodeRefused {
		t.Fatal(frame)
	}
	pipeDone(t, out, "in", true)
	pipeDone(t, out, "out", true)
}

func TestNetworkHalfCloseKeepsResponseAndReportsInputImmediately(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := listener.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
			t.Error(err)
		}
	})
	_, portText, err := net.SplitHostPort(listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	port, err := strconv.ParseUint(portText, 10, 16)
	if err != nil {
		t.Fatal(err)
	}
	respond := make(chan struct{})
	serverDone := make(chan error, 1)
	go func() {
		socket, err := listener.Accept()
		if err != nil {
			serverDone <- err
			return
		}
		defer func() { _ = socket.Close() }()
		data, err := io.ReadAll(socket)
		if err == nil && string(data) != "request" {
			err = errors.New("incorrect request")
		}
		if err == nil {
			select {
			case <-respond:
			case <-t.Context().Done():
				err = t.Context().Err()
			}
		}
		if err == nil {
			_, err = io.WriteString(socket, "response")
		}
		serverDone <- err
	}()
	uploaded := make(chan string, 1)
	s, out := pipeService(t, t.TempDir(), func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodGet {
			if _, err := io.WriteString(w, "request"); err != nil {
				t.Error(err)
			}
			return
		}
		data, err := io.ReadAll(r.Body)
		if err != nil {
			t.Error(err)
		}
		uploaded <- string(data)
	})
	operation := make(chan error, 1)
	go func() {
		operation <- s.NetOpen(t.Context(), runnerwire.NetOpen{
			StreamID: "tcp", Host: "127.0.0.1", Port: uint16(port),
			Input: runnerwire.PipeRef{ID: "in", URL: "/in"}, Output: runnerwire.PipeRef{ID: "out", URL: "/out"},
		})
	}()
	frame, ok := receiveFrame(t, out).(*runnerwire.NetOpened)
	if !ok || frame.StreamID != "tcp" {
		t.Fatal(frame)
	}
	pipeDone(t, out, "in", false)
	close(respond)
	pipeDone(t, out, "out", false)
	if err := <-operation; err != nil {
		t.Fatal(err)
	}
	if err := <-serverDone; err != nil {
		t.Fatal(err)
	}
	if got := <-uploaded; got != "response" {
		t.Fatal(got)
	}
}

func TestNetworkPipeFailureCancelsBothDirections(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = listener.Close() }) // The accepted connection is joined below.
	_, text, err := net.SplitHostPort(listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	port, err := strconv.ParseUint(text, 10, 16)
	if err != nil {
		t.Fatal(err)
	}
	ended := make(chan error, 1)
	go func() {
		socket, err := listener.Accept()
		if err != nil {
			ended <- err
			return
		}
		_, err = io.Copy(io.Discard, socket)
		ended <- errors.Join(err, socket.Close())
	}()
	s, out := pipeService(t, t.TempDir(), func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodGet {
			http.Error(w, "pipe failed", http.StatusGone)
			return
		}
		_, _ = io.Copy(io.Discard, r.Body) /* Cancellation can interrupt the upload. */
	})
	if err := s.NetOpen(
		t.Context(),
		runnerwire.NetOpen{
			StreamID: "failure",
			Host:     "127.0.0.1",
			Port:     uint16(port),
			Input:    runnerwire.PipeRef{ID: "in", URL: "/in"},
			Output:   runnerwire.PipeRef{ID: "out", URL: "/out"},
		},
	); err != nil {
		t.Fatal(err)
	}
	if _, ok := receiveFrame(t, out).(*runnerwire.NetOpened); !ok {
		t.Fatal("expected open")
	}
	seen := map[string]bool{}
	for range 2 {
		frame, ok := receiveFrame(t, out).(*runnerwire.PipeDone)
		if !ok || frame.Ok || frame.Error == nil {
			t.Fatal(frame)
		}
		seen[frame.PipeID] = true
	}
	if !seen["in"] || !seen["out"] {
		t.Fatal(seen)
	}
	if err := <-ended; err != nil {
		t.Fatal(err)
	}
}
