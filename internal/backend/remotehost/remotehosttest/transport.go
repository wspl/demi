package remotehosttest

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"net/http"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/runnerproto"
	"golang.org/x/net/websocket"
)

const fixtureToken = "fixture-token"

// binaryFrames uses the installed WebSocket codec, refusing text from a runner.
var binaryFrames = websocket.Codec{
	Marshal: websocket.Message.Marshal,
	Unmarshal: func(data []byte, kind byte, target any) error {
		if kind != websocket.BinaryFrame {
			return errors.New("the runner sent a text frame")
		}
		return websocket.Message.Unmarshal(data, kind, target)
	},
}

// socketFrames feeds complete frames to the engine without giving it socket ownership.
type socketFrames struct{ socket *websocket.Conn }

// Receive reads one binary runner frame with cancellation.
func (s socketFrames) Receive(ctx context.Context) ([]byte, error) {
	release := s.cancelIO(ctx)
	defer release()
	var frame []byte
	err := binaryFrames.Receive(s.socket, &frame)
	return frame, err
}

// Send writes one binary runner frame with cancellation.
func (s socketFrames) Send(ctx context.Context, frame []byte) error {
	release := s.cancelIO(ctx)
	defer release()
	return binaryFrames.Send(s.socket, frame)
}

// cancelIO interrupts and joins a socket operation's cancellation callback.
func (s socketFrames) cancelIO(ctx context.Context) func() {
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		defer close(done)
		if err := s.socket.SetDeadline(time.Now()); err != nil {
			slog.Debug("fixture socket deadline failed: " + err.Error())
		}
	})
	return func() {
		if !stop() {
			<-done
		}
	}
}

// publish replaces the fixture's immutable connection snapshot before notification.
func (f *RunnerFixture) publish(device remotehost.DeviceLink) {
	f.mu.Lock()
	previous := f.changed
	f.device = device
	f.changed = make(chan struct{})
	f.mu.Unlock()
	close(previous)
}

// adopt validates one hello and drives its accepted runner until both IO directions join.
func (f *RunnerFixture) adopt(socket *websocket.Conn) {
	f.mu.Lock()
	if f.closed {
		f.mu.Unlock()
		if err := socket.Close(); err != nil {
			slog.Debug("fixture socket close failed: " + err.Error())
		}
		return
	}
	f.connections.Add(1)
	f.mu.Unlock()
	defer f.connections.Done()
	defer func() {
		if err := socket.Close(); err != nil {
			slog.Debug("fixture socket close failed: " + err.Error())
		}
	}()
	socket.MaxPayloadBytes = runnerproto.MaxMessageBytes
	frames := socketFrames{socket}
	frame, err := frames.Receive(f.ctx)
	if err != nil {
		return
	}
	message, err := runnerproto.DecodeOutbound(frame)
	if err != nil {
		return
	}
	hello, ok := message.(*runnerproto.Hello)
	if !ok {
		return
	}
	permit, err := f.admission.Acquire(f.ctx)
	if err != nil {
		return
	}
	f.acceptHello(f.ctx, frames, hello, permit)
}

// pipeHTTPBody adapts request cancellation and explicit body closure to a pipe source.
type pipeHTTPBody struct {
	body     io.ReadCloser
	response *http.ResponseController
}

// Read reads the HTTP body with cancellation deadlines.
func (b pipeHTTPBody) Read(ctx context.Context, data []byte) (int, error) {
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		defer close(done)
		if err := b.response.SetReadDeadline(time.Now()); err != nil {
			slog.Debug("fixture upload deadline failed: " + err.Error())
		}
		if err := b.body.Close(); err != nil {
			slog.Debug("fixture upload close failed: " + err.Error())
		}
	})
	defer func() {
		if !stop() {
			<-done
		}
	}()
	return b.body.Read(data)
}

// Close closes the HTTP request body.
func (b pipeHTTPBody) Close(context.Context) error {
	return b.body.Close()
}

// pipeRefused maps the broker's two claim refusals to the fixture's HTTP boundary.
func pipeRefused(response http.ResponseWriter, err error) {
	status := http.StatusNotFound
	if errors.Is(err, remotehost.ErrPipeAlreadyConnected) {
		status = http.StatusConflict
	}
	pipeResponse(response, status, err.Error())
}

func (f *RunnerFixture) pipeSource(response http.ResponseWriter, request *http.Request) {
	if request.Header.Get("Authorization") != "Bearer "+fixtureToken {
		pipeResponse(response, http.StatusUnauthorized, "device token required")
		return
	}
	source, err := f.pipes.ClaimSource(request.PathValue("id"), TestDeviceID)
	if err != nil {
		pipeRefused(response, err)
		return
	}
	if err := source.Pump(
		request.Context(),
		pipeHTTPBody{request.Body, http.NewResponseController(response)},
	); err != nil {
		pipeResponse(response, http.StatusConflict, err.Error())
		return
	}
	if _, err := io.WriteString(response, "drained"); err != nil {
		slog.Debug("fixture drain reply failed: " + err.Error())
	}
}

func (f *RunnerFixture) pipeSink(response http.ResponseWriter, request *http.Request) {
	if request.Header.Get("Authorization") != "Bearer "+fixtureToken {
		pipeResponse(response, http.StatusUnauthorized, "device token required")
		return
	}
	sink, err := f.pipes.ClaimSink(request.PathValue("id"), TestDeviceID)
	if err != nil {
		pipeRefused(response, err)
		return
	}
	defer func() {
		if err := sink.Close(context.Background()); err != nil {
			slog.Debug("fixture sink close failed: " + err.Error())
		}
	}()
	if err := sink.SourceArrived(request.Context()); err != nil {
		pipeResponse(response, http.StatusConflict, err.Error())
		return
	}
	response.Header().Set("Content-Type", "application/octet-stream")
	response.Header().Set("Cache-Control", "no-store")
	response.WriteHeader(http.StatusOK)
	for {
		chunk, err := sink.Next(request.Context())
		if errors.Is(err, io.EOF) {
			return
		}
		if err != nil {
			slog.Debug("fixture download failed: " + err.Error())
			abortDownload(response)
			return
		}
		if _, err := response.Write(chunk); err != nil {
			slog.Debug("fixture download disconnected: " + err.Error())
			return
		}
		if err := http.NewResponseController(response).Flush(); err != nil {
			slog.Debug("fixture download flush failed: " + err.Error())
			return
		}
	}
}

// abortDownload truncates a failed HTTP pipe body instead of sending successful EOF.
func abortDownload(response http.ResponseWriter) {
	connection, _, err := http.NewResponseController(response).Hijack()
	if err != nil {
		slog.Debug("fixture download abort failed: " + err.Error())
		return
	}
	if err := connection.Close(); err != nil {
		slog.Debug("fixture download close failed: " + err.Error())
	}
}

// pipeResponse preserves the fixture's exact plain-text refusal body.
func pipeResponse(response http.ResponseWriter, status int, text string) {
	response.Header().Set("Content-Type", "text/plain; charset=utf-8")
	response.WriteHeader(status)
	if _, err := io.WriteString(response, text); err != nil {
		slog.Debug("fixture pipe reply not sent: " + err.Error())
	}
}

// refuseHello checks a fixture hello while the caller holds admission.
func (f *RunnerFixture) refuseHello(hello *runnerproto.Hello) *runnerproto.HelloError {
	// Admission serializes hello decisions, not the lifetime of an adopted link.
	var refusal *runnerproto.HelloError
	if hello.Protocol != runnerproto.Version {
		refusal = &runnerproto.HelloError{
			Code:   runnerproto.HelloErrorCodeUnsupportedProtocol,
			Reason: "unsupported protocol",
		}
	} else if hello.DeviceToken == nil || hello.DeviceToken.Expose() != fixtureToken {
		refusal = &runnerproto.HelloError{Code: runnerproto.HelloErrorCodeUnknownDevice, Reason: "unknown device"}
	} else {
		f.mu.Lock()
		current := f.device.Link
		f.mu.Unlock()
		if current != nil && !current.IsClosed() {
			refusal = &runnerproto.HelloError{
				Code:   runnerproto.HelloErrorCodeAlreadyConnected,
				Reason: "already connected",
			}
		}
	}
	return refusal
}

// acceptHello answers an admitted hello, transfers its permit, and serves the connection.
func (f *RunnerFixture) acceptHello(
	ctx context.Context,
	frames socketFrames,
	hello *runnerproto.Hello,
	permit *gates.Permit,
) {
	refusal := f.refuseHello(hello)
	if refusal != nil {
		permit.Release()
		frame, err := runnerproto.Encode(refusal)
		if err != nil {
			slog.Error("fixture refusal encoding failed: " + err.Error())
			return
		}
		if err := frames.Send(ctx, frame); err != nil {
			slog.Debug("fixture refusal not sent: " + err.Error())
		}
		return
	}
	welcome, err := runnerproto.Encode(&runnerproto.HelloOK{DeviceID: TestDeviceID})
	if err != nil {
		permit.Release()
		slog.Error("fixture welcome encoding failed: " + err.Error())
		return
	}
	if err := frames.Send(ctx, welcome); err != nil {
		permit.Release()
		return
	}
	identity := remotehost.HostIdentity(hello.Runner.Identity)
	link, driver := remotehost.NewLink(
		remotehost.LinkOptions{Device: TestDeviceID, Identity: identity, Pipes: f.pipes, Policy: f.policy},
	)
	driver.Tap(f.tap)
	f.publish(remotehost.DeviceLink{Link: link})
	permit.Release()
	driver.Serve(ctx, frames, frames)
	f.mu.Lock()
	if f.device.Link == link {
		previous := f.changed
		f.device = remotehost.DeviceLink{Last: new(identity)}
		f.changed = make(chan struct{})
		f.mu.Unlock()
		close(previous)
	} else {
		f.mu.Unlock()
	}
}
