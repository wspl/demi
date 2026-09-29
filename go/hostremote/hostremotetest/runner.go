package hostremotetest

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/go/hostremote"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

const fixtureToken = "fixture-token"

type FixtureOptions struct {
	Env      map[string]*string
	Commands *shell.CommandSet
	Tap      chan<- runnerproto.Outbound
}
type RunnerFixture struct {
	Process  *RunnerProcess
	Device   *TestDevice
	Policy   *CommandPolicy
	server   *httptest.Server
	handlers sync.WaitGroup
	cancel   context.CancelFunc
	close    sync.Once
}

func StartFixture(t testing.TB, options FixtureOptions) *RunnerFixture {
	t.Helper()
	Program(t, "demi-runner")
	policy := NewCommandPolicy(options.Commands)
	device := NewTestDevice(t, policy)
	ctx, cancel := context.WithCancel(context.Background())
	fixture := &RunnerFixture{Device: device, Policy: policy, cancel: cancel}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/runner", func(w http.ResponseWriter, r *http.Request) {
		fixture.handlers.Add(1)
		defer fixture.handlers.Done()
		ws, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		ws.SetReadLimit(runnerproto.MaxMessageBytes)
		transport := &socketTransport{ws: ws}
		defer transport.Close()
		frame, err := transport.Read(ctx)
		if err != nil {
			return
		}
		message, err := runnerproto.DecodeOutboundMsgpack(frame)
		if err != nil {
			return
		}
		hello, ok := message.(runnerproto.OutboundHello)
		if !ok || hello.Protocol != runnerproto.Version {
			return
		}
		if hello.DeviceToken == nil || string(*hello.DeviceToken) != fixtureToken {
			return
		}
		accepted, _ := runnerproto.EncodeInboundMsgpack(runnerproto.InboundHelloOk{DeviceID: TestDeviceID})
		if err := transport.Write(ctx, accepted); err != nil {
			return
		}
		identity := hello.Runner.Identity
		link := device.adopt(transport, shell.HostIdentity{UID: identity.UID, GID: identity.GID, Hostname: identity.Hostname, HomeDir: identity.HomeDir}, 0, options.Tap)
		select {
		case <-ctx.Done():
			link.Link.Disconnect("test over")
		case <-link.ended:
		}
		<-link.ended
	})
	mux.HandleFunc("/api/pipes/{id}", func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer "+fixtureToken {
			http.Error(w, "device token required", http.StatusUnauthorized)
			return
		}
		id := r.PathValue("id")
		switch r.Method {
		case http.MethodPut:
			source, err := device.Pipes.ClaimSource(id, TestDeviceID)
			if err != nil {
				pipeRefusal(w, err)
				return
			}
			if err := source.Pump(r.Context(), r.Body); err != nil {
				http.Error(w, err.Error(), http.StatusConflict)
				return
			}
			fmt.Fprint(w, "drained")
		case http.MethodGet:
			sink, err := device.Pipes.ClaimSink(id, TestDeviceID)
			if err != nil {
				pipeRefusal(w, err)
				return
			}
			defer sink.Close()
			if err := sink.SourceArrived(r.Context()); err != nil {
				http.Error(w, err.Error(), http.StatusConflict)
				return
			}
			w.Header().Set("Content-Type", "application/octet-stream")
			w.Header().Set("Cache-Control", "no-store")
			for {
				bytes, err := sink.Next(r.Context())
				if err == io.EOF {
					return
				}
				if err != nil {
					panic(http.ErrAbortHandler)
				}
				if _, err := w.Write(bytes); err != nil {
					return
				}
				if f, ok := w.(http.Flusher); ok {
					f.Flush()
				}
			}
		default:
			w.WriteHeader(http.StatusMethodNotAllowed)
		}
	})
	fixture.server = httptest.NewServer(mux)
	fixture.Process = StartRunner(t, fixture.server.URL, RunnerProcessOptions{Token: new(fixtureToken), Env: shell.SpawnEnv{Values: options.Env, Inherit: true}})
	t.Cleanup(fixture.Close)
	// This is only a hang guard; the hello event determines readiness.
	ready, stop := context.WithTimeout(t.Context(), 10*time.Minute)
	defer stop()
	if _, err := device.Online(ready); err != nil {
		t.Fatalf("runner not online: %v\n%s", err, fixture.Process.Output())
	}
	return fixture
}
func pipeRefusal(w http.ResponseWriter, err error) {
	status := http.StatusNotFound
	if errors.Is(err, hostremote.ErrPipeConnected) {
		status = http.StatusConflict
	}
	http.Error(w, err.Error(), status)
}
func (f *RunnerFixture) Host() *hostremote.RemoteHost             { return f.Device.Host(f.Process.Home(), nil) }
func (f *RunnerFixture) HostAt(cwd string) *hostremote.RemoteHost { return f.Device.Host(cwd, nil) }
func (f *RunnerFixture) Close() {
	f.close.Do(func() {
		f.cancel()
		f.Process.Kill()
		f.Device.Close()
		f.server.Close()
		f.handlers.Wait()
	})
}

type socketTransport struct{ ws *websocket.Conn }

func (s *socketTransport) Read(ctx context.Context) ([]byte, error) {
	kind, frame, err := s.ws.Read(ctx)
	if err != nil {
		return nil, err
	}
	if kind != websocket.MessageBinary {
		return nil, errors.New("the runner sent a text frame")
	}
	return frame, nil
}
func (s *socketTransport) Write(ctx context.Context, frame []byte) error {
	return s.ws.Write(ctx, websocket.MessageBinary, frame)
}
func (s *socketTransport) Close() error { return s.ws.CloseNow() }
