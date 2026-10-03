package usershard

import (
	"context"
	"errors"

	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

func (s *Shard) openExposeConnection(ctx context.Context, id webapi.ExposeID) (*ExposeConnection, error) {
	admission, err := expose.AdmitRelay(ctx, s.ExposeShard(), id)
	if err != nil {
		if ctx.Err() != nil || s.ctx.Err() != nil {
			return nil, expose.ErrRemoved
		}
		return nil, err
	}
	// Ownership transfers to Relay only after a successful open and registration.
	relaying := false
	defer func() {
		if !relaying {
			admission.Release()
		}
	}()
	if ctx.Err() != nil || s.ctx.Err() != nil {
		return nil, expose.ErrRemoved
	}
	select {
	case <-admission.Ending():
		return nil, expose.ErrRemoved
	default:
	}
	record := admission.Record()
	hostname, err := record.Address.Host()
	if err != nil {
		return nil, err
	}
	port, err := record.Address.Port()
	if err != nil {
		return nil, err
	}
	device := s.devices.DeviceAccess(record.Device)
	if device == nil {
		return nil, expose.ErrRelayDeviceOffline
	}
	input := s.pipes.ToDevice(string(record.Device))
	output := s.pipes.FromDevice(string(record.Device))
	defer func() {
		if !relaying {
			input.Fail("the relayed connection never opened")
			output.Fail("the relayed connection never opened")
		}
	}()
	writer, err := input.Writer()
	if err != nil {
		return nil, err
	}
	reader, err := output.Reader()
	if err != nil {
		return nil, err
	}
	lifetime, cancel := context.WithCancel(s.ctx)
	watched := make(chan struct{})
	go func() {
		defer close(watched)
		select {
		case <-admission.Ending():
			cancel()
		case <-lifetime.Done():
		}
	}()
	// The opening call owns the watcher until the registered relay takes it.
	defer func() {
		if !relaying {
			cancel()
			<-watched
		}
	}()
	stopped := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		cancel()
		close(stopped)
	})
	err = device.OpenNet(lifetime, hostname, port, input.WireRef(), output.WireRef())
	if !stop() {
		<-stopped
	}
	if ctx.Err() != nil || lifetime.Err() != nil {
		return nil, expose.ErrRemoved
	}
	select {
	case <-admission.Ending():
		return nil, expose.ErrRemoved
	default:
	}
	if err != nil {
		var failure *host.Error
		if errors.As(err, &failure) {
			if failure.Kind == host.Offline {
				return nil, expose.ErrRelayDeviceOffline
			}
			if failure.Code != "" {
				return nil, &expose.UnreachableError{Code: failure.Code}
			}
		}
		return nil, &expose.UnreachableError{Code: "unreachable"}
	}
	lease, _ := hostaccess.NewLease(lifetime)
	if !s.startWorker(func(context.Context) {
		defer func() {
			lease.Release()
			cancel()
			<-watched
		}()
		admission.Relay(lease.Context(), s.ExposeShard(), nil, func() {
			input.Fail("the relayed connection ended")
			output.Fail("the relayed connection ended")
		})
	}) {
		lease.Release()
		return nil, expose.ErrRemoved
	}
	relaying = true
	return &ExposeConnection{ToService: writer, FromService: reader, Lease: lease}, nil
}
