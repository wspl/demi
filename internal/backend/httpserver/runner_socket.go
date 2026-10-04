package httpserver

import (
	"context"
	"errors"
	"fmt"
	"net/http"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/webapiproto"
)

func (e *Server) runnerSocket(w http.ResponseWriter, r *http.Request) error {
	conn, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
	if err != nil {
		return nil
	} // Accept has already written the upgrade refusal.
	socket := runners.NewSocket(r.Context(), conn)
	defer socket.Release()
	ctx, cancel := context.WithTimeout(r.Context(), e.state.Services.Runners.HelloDeadline)
	kind, bytes, err := socket.Read(ctx)
	cancel()
	if err != nil || kind != websocket.MessageBinary {
		return nil
	}
	message, err := runnerproto.DecodeOutbound(bytes)
	if err != nil {
		return nil
	}
	hello, ok := message.(*runnerproto.Hello)
	if !ok {
		return nil
	}
	if hello.Protocol != runnerproto.Version {
		return refuseRunner(
			r.Context(),
			socket,
			runnerproto.HelloErrorCodeUnsupportedProtocol,
			fmt.Sprintf("unsupported protocol %d; this backend speaks %d", hello.Protocol, runnerproto.Version),
		)
	}
	if hello.DeviceToken == nil {
		if hello.Runner.Managed != nil && *hello.Runner.Managed {
			return refuseRunner(
				r.Context(),
				socket,
				runnerproto.HelloErrorCodeUnknownDevice,
				"a managed host presents its device token; it is never paired",
			)
		}
		return e.awaitClaim(r.Context(), socket, hello.Runner)
	}
	return e.adoptRunner(r.Context(), socket, hello)
}

func refuseRunner(ctx context.Context, socket *runners.Socket, code runnerproto.HelloErrorCode, reason string) error {
	if err := runners.Send(ctx, socket, &runnerproto.HelloError{Code: code, Reason: reason}); err != nil {
		return nil
	}
	// Closing is best effort; the owning handler always closes the transport.
	_ = socket.Close(websocket.StatusNormalClosure, "")
	return nil
}

func (e *Server) awaitClaim(ctx context.Context, socket *runners.Socket, runner runnerproto.Info) error {
	for {
		code := runners.GenerateClaimCode()
		wait := e.state.Services.Claims.Register(code, runner)
		if wait == nil {
			return nil
		}
		grant, err := func() (*runners.ClaimGrant, error) {
			defer wait.Release()
			defer e.state.Services.Claims.Withdraw(code)
			if err := runners.Send(ctx, socket, &runnerproto.ClaimPending{ClaimToken: code.Printed()}); err != nil {
				return nil, err
			}
			expiry, cancel := context.WithTimeout(ctx, e.state.Services.Runners.ClaimLifetime)
			defer cancel()
			var grant *runners.ClaimGrant
			err := watchRunner(expiry, socket, func(ctx context.Context) error {
				var err error
				grant, err = wait.Wait(ctx)
				return err
			})
			if err != nil && grant != nil {
				grant.Release()
				grant = nil
			}
			return grant, err
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

func (e *Server) handOver(
	ctx context.Context,
	socket *runners.Socket,
	runner runnerproto.Info,
	grant *runners.ClaimGrant,
) error {
	defer grant.Release()
	if err := runners.Send(ctx, socket, &runnerproto.Claimed{DeviceToken: grant.Token}); err != nil {
		return nil
	}
	shard, err := e.state.Shards.Of(ctx, grant.Device.User)
	if err != nil {
		return nil
	}
	bound := make(chan webapiproto.DeviceDTO, 1)
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

// watchRunner drops pre-acceptance frames while work waits, canceling that work
// on Close or read failure. It joins the consumer before the Socket is handed on.
func watchRunner(ctx context.Context, socket *runners.Socket, work func(context.Context) error) error {
	workCtx, cancelWork := context.WithCancel(ctx)
	defer cancelWork()
	readCtx, cancelRead := context.WithCancel(ctx)
	done := make(chan struct{})
	go func() {
		defer close(done)
		for {
			if _, _, err := socket.Read(readCtx); err != nil {
				if readCtx.Err() == nil {
					cancelWork()
				}
				return
			}
		}
	}()
	err := work(workCtx)
	cancelRead()
	<-done
	if workCtx.Err() != nil {
		return workCtx.Err()
	}
	return err
}

func (e *Server) adoptRunner(ctx context.Context, socket *runners.Socket, hello *runnerproto.Hello) error {
	var device database.DeviceRecord
	var found bool
	err := watchRunner(ctx, socket, func(ctx context.Context) error {
		if e.state.Services.Hooks != nil {
			if err := e.state.Services.Hooks.Hello(ctx, usershard.HelloTokenLookup); err != nil {
				return err
			}
		}
		var err error
		device, found, err = e.state.Services.Control.DeviceByToken(ctx, database.HashToken(hello.DeviceToken.Expose()))
		return err
	})
	if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		return nil
	}
	if err != nil {
		return refuseRunner(ctx, socket, runnerproto.HelloErrorCodeInternal, "the device could not be read")
	}
	if !found {
		return refuseRunner(ctx, socket, runnerproto.HelloErrorCodeUnknownDevice, "unknown device")
	}
	shard, err := e.state.Shards.Of(ctx, device.User)
	if err != nil {
		return nil
	}
	return shard.AdoptRunner(ctx, device, hello.Runner, socket)
}
