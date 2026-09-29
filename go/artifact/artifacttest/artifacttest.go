// Package artifacttest supports tests of code that downloads and installs
// through package artifact: a fixture HTTP server on 127.0.0.1 and the zip
// archives it serves, which [artifact.NewClientAllowingHTTP] downloads from, and
// the count of a process's waits for install locks.
package artifacttest

import (
	"archive/zip"
	"bytes"
	"context"
	"fmt"
	"net"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/go/artifact"
)

// Zip returns a zip archive of entries, each a path and its contents.
func Zip(t testing.TB, entries map[string][]byte) []byte {
	t.Helper()
	var archive bytes.Buffer
	writer := zip.NewWriter(&archive)
	for name, contents := range entries {
		file, err := writer.Create(name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := file.Write(contents); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return archive.Bytes()
}

// LockWaits counts the acquisitions of an install lock that found the lock held.
// Nothing else shows that an installer waits for another instead of installing
// beside it, so a test of concurrent installers watches it.
type LockWaits struct {
	waits atomic.Int64
	// events holds a token for each wait not yet received.
	once   sync.Once
	events chan struct{}
}

// eventsBuffer is how many waits a test may leave unreceived.
const eventsBuffer = 1024

func (w *LockWaits) channel() chan struct{} {
	w.once.Do(func() { w.events = make(chan struct{}, eventsBuffer) })
	return w.events
}

// Watch returns ctx changed so that the install locks acquired with it are
// counted.
func (w *LockWaits) Watch(ctx context.Context) context.Context {
	return artifact.ObserveLockWaits(ctx, func() {
		w.waits.Add(1)
		select {
		case w.channel() <- struct{}{}:
		default:
		}
	})
}

// Count returns how many acquisitions have found the lock held and waited.
func (w *LockWaits) Count() int { return int(w.waits.Load()) }

// Waited receives once for each wait, so a test waits for an installer to wait
// instead of for time.
func (w *LockWaits) Waited() <-chan struct{} { return w.channel() }

// An Answer is what a fixture server answers for one path.
type Answer struct {
	Status int
	Body   []byte
	// Unsized leaves out the answer's length: the body ends when the connection
	// closes.
	Unsized bool
	// Location is the answer's Location header, when it has one.
	Location string
	// Hold, when not nil, is what the server waits for after the request, before
	// it answers: the test closes it, or the server stops.
	Hold <-chan struct{}
}

// OK returns the answer 200 with body and its length, at once.
func OK(body []byte) Answer {
	return Answer{Status: 200, Body: body}
}

// A Server is a fixture HTTP server on 127.0.0.1: it answers each path it was
// given with that path's answer and any other with 404, closes each connection
// after its answer, and counts the requests.
type Server struct {
	base     string
	listener net.Listener
	requests atomic.Int64
	arrived  chan struct{}
	mu       sync.Mutex
	last     string
	done     chan struct{}
	stop     sync.Once
	wait     sync.WaitGroup
}

// Start starts a server with answers, by path, and stops it when the test ends.
func Start(t testing.TB, answers map[string]Answer) *Server {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	s := &Server{base: "http://" + listener.Addr().String(), listener: listener, arrived: make(chan struct{}, 64), done: make(chan struct{})}
	s.wait.Add(1)
	go func() {
		defer s.wait.Done()
		for {
			connection, err := listener.Accept()
			if err != nil {
				return
			}
			s.wait.Add(1)
			go func() {
				defer s.wait.Done()
				s.serve(connection, answers)
			}()
		}
	}()
	t.Cleanup(s.Stop)
	return s
}

// Stop stops the server, before the test ends.
func (s *Server) Stop() {
	s.stop.Do(func() {
		close(s.done)
		s.listener.Close()
		s.wait.Wait()
	})
}

// URL returns the URL of path on this server.
func (s *Server) URL(path string) string { return s.base + path }

// Arrived receives once for each request the server reads, so a test waits for
// a request instead of for time.
func (s *Server) Arrived() <-chan struct{} { return s.arrived }

// LastRequest returns the head of the request the server read last, as the
// client sent it.
func (s *Server) LastRequest() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.last
}

// Requests returns how many requests the server has read.
func (s *Server) Requests() int { return int(s.requests.Load()) }

func (s *Server) serve(connection net.Conn, answers map[string]Answer) {
	defer connection.Close()
	request := make([]byte, 4096)
	// A connection that fails before its request gets no answer.
	count, err := connection.Read(request)
	if err != nil {
		return
	}
	s.mu.Lock()
	s.last = string(request[:count])
	s.mu.Unlock()
	s.requests.Add(1)
	select {
	case s.arrived <- struct{}{}:
	default:
	}
	// The request line is "GET <path> HTTP/1.1".
	fields := strings.Fields(string(request[:count]))
	answer := Answer{Status: 404}
	if len(fields) > 1 {
		if found, ok := answers[fields[1]]; ok {
			answer = found
		}
	}
	if answer.Hold != nil {
		select {
		case <-answer.Hold:
		case <-s.done:
			return
		}
	}
	length := ""
	if !answer.Unsized {
		length = fmt.Sprintf("content-length: %d\r\n", len(answer.Body))
	}
	if answer.Location != "" {
		length += "location: " + answer.Location + "\r\n"
	}
	// The client may hang up first, as on a failed check; nothing waits for the
	// answer then.
	fmt.Fprintf(connection, "HTTP/1.1 %d Fixture\r\n%sconnection: close\r\n\r\n", answer.Status, length)
	connection.Write(answer.Body)
}
