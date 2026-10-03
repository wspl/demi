package usershard

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"

	"github.com/coder/websocket"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

type shardPolicy struct {
	shard  *Shard
	device webapi.DeviceID
}

func (p shardPolicy) AdmitCall(job remotehost.JobOrigin) error {
	conversation, ok := runners.ConversationOf(job.Host)
	if !ok {
		return errors.New("rpc requires a live job dispatched to this device")
	}
	if conversation != job.Context.Conversation {
		return errors.New("rpc job belongs to another conversation")
	}
	return nil
}
func (p shardPolicy) Dispatch(ctx context.Context, job remotehost.JobOrigin, invocation host.RPCInvocation, port host.RPCPort) (uint8, error) {
	return p.shard.commands.Dispatch(ctx, job, invocation, port)
}
func (p shardPolicy) Storage(ctx context.Context, job remotehost.JobOrigin, op host.StorageOp) (host.StorageReply, error) {
	if job.Caller == nil {
		return nil, &host.PortError{Kind: host.StorageRefused, Message: "the job has no command storage"}
	}
	conversation, err := webapi.ParseConversationID(job.Context.Conversation)
	if err != nil {
		return nil, &host.PortError{Kind: host.StorageRefused, Message: "the job belongs to no conversation", Err: err}
	}
	return p.shard.agent.CommandStorage(ctx, hostaccess.RootOf(conversation), *job.Caller, op)
}
func (p shardPolicy) GrowVolume(ctx context.Context, volume runnerwire.VolumeName, bytes uint64) error {
	err := cloud.GrowVolume(ctx, p.shard, p.device, volume, bytes)
	if err != nil {
		slog.WarnContext(ctx, "volume growth refused", "device", p.device, "volume", volume, "bytes", bytes, "error", err)
	}
	return err
}
func (p shardPolicy) ReserveNumbers(ctx context.Context, conversation string, sequence commandwire.ServiceSequence, count uint32) (uint64, error) {
	id, err := webapi.ParseConversationID(conversation)
	if err != nil {
		return 0, fmt.Errorf("no conversation %s: %w", conversation, err)
	}
	hosts, err := hostaccess.Reachable(ctx, p.shard, id)
	if err != nil {
		return 0, err
	}
	reaches := false
	for _, reachable := range hosts {
		if reachable.Device == p.device {
			reaches = true
			break
		}
	}
	if !reaches {
		return 0, errors.New("the conversation does not reach this device")
	}
	var value uint64
	err = p.shard.ConversationDB(id).Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		value, err = database.ReserveNumbers(ctx, tx, core.Sequence(sequence), count)
		return err
	})
	return value, err
}

func (s *Shard) adopt(ctx context.Context, device database.DeviceRecord, runner runnerwire.RunnerInfo, socket *websocket.Conn, bound chan<- webapi.DeviceDTO) error {
	// Closing an already broken socket needs no recovery.
	defer func() { _ = socket.CloseNow() }()
	if bound != nil {
		defer close(bound)
	}
	// Runner drivers outlive product draining, so shutdown joins them last.
	s.mu.Lock()
	if s.closing {
		s.mu.Unlock()
		return &ShardUnavailable{Kind: ShardClosing}
	}
	s.runners.Add(1)
	s.mu.Unlock()
	defer s.runners.Done()
	ctx, cancel := context.WithCancel(ctx)
	stopped := make(chan struct{})
	stop := context.AfterFunc(s.runnerCtx, func() {
		cancel()
		close(stopped)
	})
	defer func() {
		if !stop() {
			<-stopped
		}
	}()
	defer cancel()
	if bound == nil && s.services.Hooks != nil {
		if err := s.services.Hooks.Hello(ctx, HelloBind); err != nil {
			return err
		}
	}
	order, err := s.runnerOrder.Acquire(ctx, device.ID)
	if err != nil {
		return err
	}
	if err := s.devices.Settled(ctx, device.ID); err != nil {
		order.Release()
		return err
	}
	if s.devices.Online(device.ID) {
		order.Release()
		slog.WarnContext(ctx, fmt.Sprintf("runner hello refused (already_connected): device %s already has a live connection [%s, %s]", device.ID, runner.Name, runner.Platform), "device", device.ID)
		return runners.Send(ctx, socket, &runnerwire.HelloError{Code: runnerwire.HelloErrorCodeAlreadyConnected, Reason: fmt.Sprintf("device %s already has a live connection", device.ID)})
	}
	s.mu.Lock()
	closing := s.closing
	s.mu.Unlock()
	if closing {
		order.Release()
		return &ShardUnavailable{Kind: ShardClosing}
	}
	link, driver := remotehost.NewLink(remotehost.LinkOptions{Device: string(device.ID), Identity: remotehost.HostIdentity(runner.Identity), Pipes: s.pipes, Policy: shardPolicy{shard: s, device: device.ID}, Ping: s.services.Runners.Ping})
	seen := runners.NewLastSeen(s.Control(), s.services.Sync.Of(s.user))
	serving := s.devices.Bind(device.ID, link, driver, seen)
	order.Release()
	s.startWorker(func(ctx context.Context) { seen.Touch(ctx, device.ID) })
	if bound != nil {
		bound <- s.devices.DTO(device)
	} else {
		if err := runners.Send(ctx, socket, &runnerwire.HelloOK{DeviceID: string(device.ID)}); err != nil {
			serving.Close(context.WithoutCancel(ctx))
			return err
		}
	}
	serving.Serve(ctx, socket)
	return nil
}
