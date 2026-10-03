package edge

import (
	"context"
	"errors"
	"fmt"
	"net/http"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) runnerSocket(w http.ResponseWriter, r *http.Request) error {
	socket, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
	if err != nil {
		return nil
	} // Accept has already written the upgrade refusal.
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = socket.CloseNow() }()
	socket.SetReadLimit(runnerwire.MaxMessageBytes)
	ctx, cancel := context.WithTimeout(r.Context(), e.state.Services.Runners.HelloDeadline)
	kind, bytes, err := socket.Read(ctx)
	cancel()
	if err != nil || kind != websocket.MessageBinary {
		return nil
	}
	message, err := runnerwire.DecodeOutbound(bytes)
	if err != nil {
		return nil
	}
	hello, ok := message.(*runnerwire.Hello)
	if !ok {
		return nil
	}
	if hello.Protocol != runnerwire.Version {
		return refuseRunner(r.Context(), socket, runnerwire.HelloErrorCodeUnsupportedProtocol, fmt.Sprintf("unsupported protocol %d; this backend speaks %d", hello.Protocol, runnerwire.Version))
	}
	if hello.DeviceToken == nil {
		if hello.Runner.Managed != nil && *hello.Runner.Managed {
			return refuseRunner(r.Context(), socket, runnerwire.HelloErrorCodeUnknownDevice, "a managed host presents its device token; it is never paired")
		}
		return e.awaitClaim(r.Context(), socket, hello.Runner)
	}
	// Transport closure cancels the connection context during this lookup.
	// The concrete websocket handoff API cannot transfer a pending frame read;
	// immediate Close-frame observation and discarding repeated hellos require
	// a shard API that accepts an edge-owned reader (see b-edge's report).
	if e.state.Services.Hooks != nil {
		if err := e.state.Services.Hooks.Hello(r.Context(), usershard.HelloTokenLookup); err != nil {
			return nil
		}
	}
	device, err := e.state.Services.Control.DeviceByToken(r.Context(), database.HashToken(hello.DeviceToken.Expose()))
	if err != nil {
		return refuseRunner(r.Context(), socket, runnerwire.HelloErrorCodeInternal, "the device could not be read")
	}
	if device == nil {
		return refuseRunner(r.Context(), socket, runnerwire.HelloErrorCodeUnknownDevice, "unknown device")
	}
	shard, err := e.state.Shards.Of(r.Context(), device.User)
	if err != nil {
		return nil
	}
	return shard.AdoptRunner(r.Context(), *device, hello.Runner, socket)
}
func refuseRunner(ctx context.Context, socket *websocket.Conn, code runnerwire.HelloErrorCode, reason string) error {
	if err := runners.Send(ctx, socket, &runnerwire.HelloError{Code: code, Reason: reason}); err != nil {
		return nil
	}
	// Closing is best effort; the owning handler always closes the transport.
	_ = socket.Close(websocket.StatusNormalClosure, "")
	return nil
}
func (e *Edge) awaitClaim(ctx context.Context, socket *websocket.Conn, runner runnerwire.RunnerInfo) error {
	for {
		code := runners.GenerateClaimCode()
		wait := e.state.Services.Claims.Register(code, runner)
		if wait == nil {
			return nil
		}
		grant, err := func() (*runners.ClaimGrant, error) {
			defer wait.Release()
			defer e.state.Services.Claims.Withdraw(code)
			if err := runners.Send(ctx, socket, &runnerwire.ClaimPending{ClaimToken: code.Printed()}); err != nil {
				return nil, err
			}
			expiry, cancel := context.WithTimeout(ctx, e.state.Services.Runners.ClaimLifetime)
			defer cancel()
			return wait.Wait(expiry)
		}()
		if errors.Is(err, context.DeadlineExceeded) && ctx.Err() == nil {
			continue
		}
		if err != nil || grant == nil {
			return nil
		}
		return e.handOver(ctx, socket, runner, grant)
	}
}
func (e *Edge) handOver(ctx context.Context, socket *websocket.Conn, runner runnerwire.RunnerInfo, grant *runners.ClaimGrant) error {
	defer grant.Release()
	if err := runners.Send(ctx, socket, &runnerwire.Claimed{DeviceToken: grant.Token}); err != nil {
		return nil
	}
	shard, err := e.state.Shards.Of(ctx, grant.Device.User)
	if err != nil {
		return nil
	}
	bound := make(chan webapi.DeviceDTO, 1)
	done := make(chan struct{})
	go func() {
		defer close(done)
		if device, ok := <-bound; ok {
			grant.Bound(device)
		}
	}()
	err = shard.AdoptClaimed(ctx, grant.Device, runner, socket, bound)
	<-done
	return err
}
