package usershard

import (
	"context"
	"errors"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

func (s *Shard) connectExpose(ctx context.Context, id webapi.ExposeID) (*ExposeConnection, error) {
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
	connection, err := s.connectAdmittedExpose(ctx, admission)
	if err == nil {
		relaying = true
	}
	return connection, err
}

// exposeOpenError translates host opening failures into expose refusals.
func exposeOpenError(err error) error {
	var failure *host.Error
	if errors.As(err, &failure) {
		if failure.Kind == host.Offline {
			return expose.ErrRelayDeviceOffline
		}
		if failure.Code != "" {
			return &expose.UnreachableError{Code: failure.Code}
		}
	}
	return &expose.UnreachableError{Code: "unreachable"}
}

// exposeDestination resolves the exposed address and connected device.
func (s *Shard) exposeDestination(record database.ExposeRecord) (*remotehost.Host, string, uint16, error) {
	hostname, err := record.Address.Host()
	if err != nil {
		return nil, "", 0, err
	}
	port, err := record.Address.Port()
	if err != nil {
		return nil, "", 0, err
	}
	device := s.devices.DeviceAccess(record.Device)
	if device == nil {
		return nil, "", 0, expose.ErrRelayDeviceOffline
	}
	return device, hostname, port, nil
}

// openExposedNetwork opens the stream while observing requester and expose cancellation.
func openExposedNetwork(ctx context.Context, opening exposeOpening) error {
	stopped := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		opening.cancel()
		close(stopped)
	})
	err := opening.device.OpenNet(
		opening.lifetime,
		opening.hostname,
		opening.port,
		opening.input.WireRef(),
		opening.output.WireRef(),
	)
	if !stop() {
		<-stopped
	}
	if ctx.Err() != nil || opening.lifetime.Err() != nil {
		return expose.ErrRemoved
	}
	select {
	case <-opening.admission.Ending():
		return expose.ErrRemoved
	default:
	}
	if err != nil {
		return exposeOpenError(err)
	}
	return nil
}

type exposeOpening struct {
	lifetime  context.Context
	cancel    context.CancelFunc
	admission *expose.RelayAdmission
	device    *remotehost.Host
	hostname  string
	port      uint16
	input     *remotehost.Pipe
	output    *remotehost.Pipe
}

// watchExpose ends the network lifetime when its expose is removed.
func watchExpose(
	ctx context.Context,
	admission *expose.RelayAdmission,
	cancel context.CancelFunc,
	watched chan<- struct{},
) {
	defer close(watched)
	select {
	case <-admission.Ending():
		cancel()
	case <-ctx.Done():
	}
}

// connectAdmittedExpose owns the opening pipes and watcher until a relay worker takes them.
func (s *Shard) connectAdmittedExpose(
	ctx context.Context,
	admission *expose.RelayAdmission,
) (*ExposeConnection, error) {
	relaying := false
	if ctx.Err() != nil || s.ctx.Err() != nil {
		return nil, expose.ErrRemoved
	}
	select {
	case <-admission.Ending():
		return nil, expose.ErrRemoved
	default:
	}
	record := admission.Record()
	device, hostname, port, err := s.exposeDestination(record)
	if err != nil {
		return nil, err
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
	go watchExpose(lifetime, admission, cancel, watched)
	// The opening call owns the watcher until the registered relay takes it.
	defer func() {
		if !relaying {
			cancel()
			<-watched
		}
	}()
	opening := exposeOpening{
		lifetime:  lifetime,
		cancel:    cancel,
		admission: admission,
		device:    device,
		hostname:  hostname,
		port:      port,
		input:     input,
		output:    output,
	}
	if err := openExposedNetwork(ctx, opening); err != nil {
		return nil, err
	}
	lease, err := s.startExposeRelay(opening, watched)
	if err != nil {
		return nil, err
	}
	relaying = true
	return &ExposeConnection{ToService: writer, FromService: reader, Lease: lease}, nil
}

// startExposeRelay transfers the watcher and pipe ownership to a shard worker.
func (s *Shard) startExposeRelay(opening exposeOpening, watched <-chan struct{}) (*hostaccess.Lease, error) {
	lease, _ := hostaccess.NewLease(opening.lifetime)
	if !s.startWorker(func(context.Context) {
		defer func() {
			lease.Release()
			opening.cancel()
			<-watched
		}()
		opening.admission.Relay(lease.Context(), s.ExposeShard(), nil, func() {
			opening.input.Fail("the relayed connection ended")
			opening.output.Fail("the relayed connection ended")
		})
	}) {
		lease.Release()
		return nil, expose.ErrRemoved
	}
	return lease, nil
}
