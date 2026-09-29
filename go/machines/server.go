//go:build linux

package machines

import (
	"bufio"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log/slog"
	"net"
	"os"
	"path/filepath"
	"reflect"
	"sync"
	"time"

	"github.com/wspl/demi/go/machinesproto"
)

// The socket server (docs/cloud/managed-hosts.md § Control and ownership): one
// Unix socket, any number of backend connections, every request handed to one
// [Service]. Requests on a connection run concurrently and are answered in
// completion order; a death event goes to every connection. The socket's owner
// and group are the boundary: there is no authentication and no TCP listener.
//
// A connection has one goroutine that reads its lines, one goroutine per request,
// and one goroutine that owns the socket's writes.

const (
	// outboxLines is how many lines wait for one connection's socket. The backend
	// reads its replies as they come, so a full queue only delays the requests
	// that wait on it.
	outboxLines = 64
	// acceptBackoff is the pause after a failed accept.
	acceptBackoff = 100 * time.Millisecond
)

// A Service serves the socket's requests.
type Service interface {
	// Handle runs one call and returns its result as the ok reply carries it:
	// any value that encodes as JSON. ctx is not cancelled when the connection
	// closes: the operation completes either way.
	Handle(ctx context.Context, call machinesproto.Call) (any, error)
}

// A Socket is the bound socket; [Serve] removes its file when it stops.
type Socket struct {
	listener net.Listener
	path     string
}

// Bind binds path with mode 0660, so the service's group may connect. Its parent
// is created; a socket an earlier process left there is replaced, and any other
// file is refused rather than deleted.
func Bind(path string) (*Socket, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0o777); err != nil {
		return nil, err
	}
	switch info, err := os.Lstat(path); {
	case err == nil && info.Mode().Type() == fs.ModeSocket:
		if err := os.Remove(path); err != nil {
			return nil, err
		}
	case err == nil:
		return nil, fmt.Errorf("%s exists and is not a socket", path)
	case !errors.Is(err, fs.ErrNotExist):
		return nil, err
	}
	listener, err := net.Listen("unix", path)
	if err != nil {
		return nil, fmt.Errorf("failed to listen on %s: %w", path, err)
	}
	if err := os.Chmod(path, 0o660); err != nil {
		listener.Close()
		return nil, err
	}
	return &Socket{listener: listener, path: path}, nil
}

// InFlight is the requests still running after the server stopped. Their
// operations complete, and their replies are discarded.
type InFlight struct {
	requests *sync.WaitGroup
}

// Wait waits for every request to finish.
func (f *InFlight) Wait() {
	f.requests.Wait()
}

// A connection is one backend connection.
type connection struct {
	conn net.Conn
	out  chan []byte
	// closed is closed when the connection is done, so nothing waits to queue a
	// line no one will write.
	closed chan struct{}
	once   sync.Once
}

// finish marks the connection done: the writer sends what is queued and ends.
func (c *connection) finish() {
	c.once.Do(func() { close(c.closed) })
}

// send queues a line for the socket, unless the connection has closed.
func (c *connection) send(line []byte) {
	select {
	case <-c.closed:
		return
	default:
	}
	select {
	case c.out <- line:
	case <-c.closed:
	}
}

type server struct {
	service Service
	// ctx carries the values of the server's context, not its cancellation:
	// requests run to their end.
	ctx      context.Context
	requests sync.WaitGroup

	mu          sync.Mutex
	connections map[uint64]*connection
	handlers    sync.WaitGroup
}

// Serve serves socket until stopping is done, then stops accepting, drops every
// connection and removes the socket file. Requests still running are returned:
// the caller waits for them once the service no longer admits work, so none of
// them is cut short. deaths carries the devices whose sandbox exited; Serve
// tells every connection, and keeps taking them, telling no one, until the
// channel is closed.
func Serve(stopping context.Context, socket *Socket, service Service, deaths <-chan machinesproto.DeviceID) *InFlight {
	s := &server{
		service:     service,
		ctx:         context.WithoutCancel(stopping),
		connections: make(map[uint64]*connection),
	}
	s.requests.Add(1)
	go s.forwardDeaths(deaths)
	stop := context.AfterFunc(stopping, func() { socket.listener.Close() })
	defer stop()
	var next uint64
	for {
		conn, err := socket.listener.Accept()
		if err != nil {
			if stopping.Err() != nil {
				break
			}
			slog.Warn("machines: accepting a connection failed: " + err.Error())
			// A failure such as a lack of open files lasts a while; retrying at
			// once would only spin.
			select {
			case <-time.After(acceptBackoff):
			case <-stopping.Done():
			}
			continue
		}
		c := &connection{conn: conn, out: make(chan []byte, outboxLines), closed: make(chan struct{})}
		id := next
		next++
		s.mu.Lock()
		s.connections[id] = c
		s.mu.Unlock()
		s.handlers.Add(1)
		go func() {
			defer s.handlers.Done()
			s.serveConnection(c)
			s.mu.Lock()
			delete(s.connections, id)
			s.mu.Unlock()
		}()
	}
	s.mu.Lock()
	for _, c := range s.connections {
		c.finish()
		c.conn.Close()
	}
	s.mu.Unlock()
	s.handlers.Wait()
	if err := os.Remove(socket.path); err != nil && !errors.Is(err, fs.ErrNotExist) {
		slog.Warn("machines: removing the socket failed: " + err.Error())
	}
	return &InFlight{requests: &s.requests}
}

// forwardDeaths tells every connection of each device whose sandbox exited, each
// through its own queue, so a connection that lags delays only itself. It ends
// when deaths is closed.
func (s *server) forwardDeaths(deaths <-chan machinesproto.DeviceID) {
	defer s.requests.Done()
	for device := range deaths {
		line, err := machinesproto.EncodeResponse(machinesproto.Death{DeviceID: string(device)})
		if err != nil {
			slog.Error("machines: " + err.Error())
			continue
		}
		s.mu.Lock()
		for _, c := range s.connections {
			s.requests.Add(1)
			go func() {
				defer s.requests.Done()
				c.send(line)
			}()
		}
		s.mu.Unlock()
	}
}

// serveConnection reads a connection's requests until a bad line or its end, and
// writes its replies. A bad line ends the connection; the lines behind it are not
// served.
func (s *server) serveConnection(c *connection) {
	writing := make(chan struct{})
	go func() {
		defer close(writing)
		s.writeLines(c)
	}()
	s.readRequests(c)
	// The reader is done: the writer sends what is queued and ends once no
	// request can add to it.
	c.finish()
	<-writing
	c.conn.Close()
}

var errTooLong = errors.New("a line is longer than the limit")

// readLine reads one line, its newline and a carriage return before it removed,
// of at most limit bytes.
func readLine(reader *bufio.Reader, limit int) ([]byte, error) {
	var line []byte
	for {
		chunk, err := reader.ReadSlice('\n')
		line = append(line, chunk...)
		if len(line) > limit+2 {
			return nil, errTooLong
		}
		switch {
		case err == nil:
			line = line[:len(line)-1]
			if len(line) > 0 && line[len(line)-1] == '\r' {
				line = line[:len(line)-1]
			}
			if len(line) > limit {
				return nil, errTooLong
			}
			return line, nil
		case errors.Is(err, bufio.ErrBufferFull):
		case errors.Is(err, io.EOF):
			// The last line of a stream has no newline.
			if len(line) == 0 {
				return nil, io.EOF
			}
			if line[len(line)-1] == '\r' {
				line = line[:len(line)-1]
			}
			if len(line) > limit {
				return nil, errTooLong
			}
			return line, nil
		default:
			return nil, err
		}
	}
}

func (s *server) readRequests(c *connection) {
	reader := bufio.NewReaderSize(c.conn, 64*1024)
	for {
		line, err := readLine(reader, machinesproto.MaxLineBytes)
		switch {
		case errors.Is(err, io.EOF):
			return
		case errors.Is(err, errTooLong):
			slog.Warn("machines: dropping a connection over a bad frame: " + err.Error())
			return
		case err != nil:
			slog.Debug("machines: connection error: " + err.Error())
			return
		}
		if len(line) == 0 {
			continue
		}
		request, err := machinesproto.DecodeRequest(line)
		if err != nil {
			slog.Warn("machines: dropping a connection over a bad frame: " + err.Error())
			return
		}
		s.requests.Add(1)
		go func() {
			defer s.requests.Done()
			s.answer(c, request)
		}()
	}
}

// writeLines owns the connection's writes: it writes each queued line, and after
// the connection is done, what is still queued.
func (s *server) writeLines(c *connection) {
	write := func(line []byte) bool {
		if _, err := c.conn.Write(line); err != nil {
			slog.Debug("machines: connection error: " + err.Error())
			// The socket broke; there is no one left to answer.
			c.finish()
			c.conn.Close()
			return false
		}
		return true
	}
	for {
		select {
		case line := <-c.out:
			if !write(line) {
				return
			}
		case <-c.closed:
			for {
				select {
				case line := <-c.out:
					if !write(line) {
						return
					}
				default:
					return
				}
			}
		}
	}
}

// answer runs one request and queues its reply, unless its connection has closed:
// the operation completes either way.
func (s *server) answer(c *connection, request machinesproto.Request) {
	operation := reflect.TypeOf(request.Call).Name()
	var response machinesproto.Response
	result, err := s.service.Handle(s.ctx, request.Call)
	if err == nil {
		var encoded jsontext.Value
		if encoded, err = json.Marshal(result); err == nil {
			response = machinesproto.OK{ID: request.ID, Result: encoded}
		}
	}
	if err != nil {
		slog.Warn(fmt.Sprintf("machines: %s failed: %s", operation, Chain(err)))
		response = machinesproto.Failure{ID: request.ID, Message: err.Error()}
	}
	line, encodeErr := machinesproto.EncodeResponse(response)
	if encodeErr != nil {
		slog.Error("machines: " + encodeErr.Error())
		return
	}
	c.send(line)
}
