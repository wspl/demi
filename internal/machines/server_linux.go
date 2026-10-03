//go:build linux

package machines

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/wspl/demi/internal/machinewire"
)

// Service runs one call and returns the JSON value carried in its ok reply.
type Service interface {
	Handle(context.Context, machinewire.MachineCall) (json.RawMessage, error)
}

// Socket owns a bound Unix socket. Serve consumes it.
type Socket struct{ listener *net.UnixListener }

// BindSocket binds a mode-0660 socket, replacing only stale socket files.
func BindSocket(ctx context.Context, path string) (*Socket, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o777); err != nil {
		return nil, err
	}
	info, err := os.Lstat(path)
	if err == nil {
		if info.Mode()&os.ModeSocket == 0 {
			return nil, fmt.Errorf("%s exists and is not a socket", path)
		}
		if err = os.Remove(path); err != nil {
			return nil, err
		}
	} else if !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}
	listener, err := net.ListenUnix("unix", &net.UnixAddr{Name: path, Net: "unix"})
	if err != nil {
		return nil, err
	}
	if err = os.Chmod(path, 0o660); err != nil {
		_ = listener.Close()
		return nil, err
	}
	return &Socket{listener: listener}, nil
}

// InFlight owns requests still completing after the socket server stops.
type InFlight struct{ requests sync.WaitGroup }

// Wait joins every request. Cancellation does not cut an operation short.
func (f *InFlight) Wait(_ context.Context) { f.requests.Wait() }

type connection struct {
	socket *net.UnixConn
	out    chan []byte
	ctx    context.Context
	cancel context.CancelFunc
}

// Serve accepts requests until ctx is canceled, then closes and joins connections.
// The caller closes manager admission before joining the returned requests.
func Serve(ctx context.Context, socket *Socket, service Service, deaths <-chan machinewire.DeviceID) *InFlight {
	flight := &InFlight{}
	var mu sync.Mutex
	clients := map[*connection]struct{}{}
	var owned sync.WaitGroup
	done := make(chan struct{})
	owned.Go(func() {
		select {
		case <-ctx.Done():
		case <-done:
		}
		// Closing stops accept and removes the socket; no buffered data is held.
		_ = socket.listener.Close()
	})
	owned.Go(func() { broadcastDeaths(ctx, done, deaths, &mu, clients, flight) })
	var connections sync.WaitGroup
	for ctx.Err() == nil {
		conn, err := socket.listener.AcceptUnix()
		if err != nil {
			if ctx.Err() != nil {
				break
			}
			slog.Warn("machines: accepting a connection failed: " + err.Error())
			timer := time.NewTimer(100 * time.Millisecond)
			select {
			case <-ctx.Done():
			case <-timer.C:
			}
			timer.Stop()
			continue
		}
		cctx, cancel := context.WithCancel(ctx)
		c := &connection{socket: conn, out: make(chan []byte, 64), ctx: cctx, cancel: cancel}
		mu.Lock()
		clients[c] = struct{}{}
		mu.Unlock()
		connections.Go(func() {
			c.run(c.ctx, service, flight)
			mu.Lock()
			delete(clients, c)
			mu.Unlock()
		})
	}
	close(done)
	owned.Wait()
	connections.Wait()
	return flight
}

func (c *connection) send(ctx context.Context, line []byte) {
	select {
	case c.out <- line:
	case <-ctx.Done():
	}
}

func (c *connection) run(ctx context.Context, service Service, flight *InFlight) {
	defer c.cancel()
	var owned sync.WaitGroup
	owned.Go(func() {
		<-c.ctx.Done()
		// Shutdown discards queued replies and interrupts both socket halves.
		_ = c.socket.Close()
	})
	owned.Go(func() {
		for {
			select {
			case <-c.ctx.Done():
				return
			case line := <-c.out:
				if _, err := c.socket.Write(line); err != nil {
					c.cancel()
					return
				}
			}
		}
	})
	reader := bufio.NewScanner(c.socket)
	reader.Buffer(make([]byte, 4096), machinewire.MaxLineBytes+2)
	for reader.Scan() {
		line := reader.Bytes()
		if len(line) == 0 {
			continue
		}
		request, err := machinewire.DecodeRequest(line)
		if err != nil {
			break
		}
		flight.requests.Go(func() {
			result, err := service.Handle(context.WithoutCancel(ctx), request.Call)
			var response machinewire.MachineResponse = &machinewire.OK{ID: request.ID, Result: result}
			if err != nil {
				slog.Warn("machines: " + request.Call.Name() + " failed: " + ErrorChain(err))
				response = &machinewire.ErrorResponse{ID: request.ID, Message: err.Error()}
			}
			line, err := machinewire.EncodeLine(response)
			if err != nil {
				slog.Error(err.Error())
				return
			}
			c.send(c.ctx, line)
		})
	}
	c.cancel()
	owned.Wait()
}

// broadcastDeaths snapshots connections under the mutex and sends after releasing it.
func broadcastDeaths(
	ctx context.Context,
	done <-chan struct{},
	deaths <-chan machinewire.DeviceID,
	mu *sync.Mutex,
	clients map[*connection]struct{},
	flight *InFlight,
) {
	for {
		select {
		case <-ctx.Done():
			return
		case <-done:
			return
		case device, ok := <-deaths:
			if !ok {
				return
			}
			line, err := machinewire.EncodeLine(&machinewire.Death{DeviceID: string(device)})
			if err != nil {
				slog.Error(err.Error())
				continue
			}
			mu.Lock()
			snapshot := make([]*connection, 0, len(clients))
			for c := range clients {
				snapshot = append(snapshot, c)
			}
			mu.Unlock()
			for _, c := range snapshot {
				flight.requests.Go(func() { c.send(c.ctx, line) })
			}
		}
	}
}
