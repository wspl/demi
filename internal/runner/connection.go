package runner

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"sync"
	"time"

	"github.com/wspl/demi/internal/runner/host"
	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerproto"
)

// errRegistrationRefused reports that the backend refused this runner's registration.
var errRegistrationRefused = errors.New("backend rejected runner registration")

// errConnectionLost reports that the backend connection ended without a local error.
var errConnectionLost = errors.New("backend connection lost")

type connectionWork struct {
	installation uint64
	installed    *jobs.Installed
	err          error
}

// connection owns all work and leases that end with one backend socket.
type connection struct {
	registration     *registration
	transport        *transport
	ctx              context.Context
	cancel           context.CancelFunc
	handle           *jobs.Connection
	requests         <-chan jobs.Request
	table            *jobs.Table
	directories      *jobs.Directories
	contexts         *jobs.ContextTable
	installation     *jobs.Installation
	installed        *jobs.Installed
	generation       uint64
	relay            *jobs.Relay
	host             *host.Service
	streams          *jobs.ServiceStreams
	volumes          *host.Volumes
	work             sync.WaitGroup
	results          chan connectionWork
	finished         chan jobs.WorkID
	tasksChanged     chan struct{}
	installsChanged  chan struct{}
	installsReported bool
}

func (r *registration) serve(ctx context.Context, t *transport) (err error) {
	lifetime, cancel := context.WithCancel(ctx)
	c := &connection{
		registration:    r,
		transport:       t,
		ctx:             lifetime,
		cancel:          cancel,
		installation:    &jobs.Installation{},
		results:         make(chan connectionWork),
		finished:        make(chan jobs.WorkID),
		tasksChanged:    make(chan struct{}, 1),
		installsChanged: make(chan struct{}, 1),
	}
	c.handle, c.requests = jobs.NewConnection(lifetime, t.control)
	c.directories = jobs.OpenDirectories(lifetime, r.options.jobRoot)
	c.contexts = jobs.NewContextTable(r.contexts)
	c.table = jobs.NewTable(
		lifetime,
		jobs.Config{
			Output:      t.output,
			Directories: c.directories,
			Pipes:       r.pipes,
			Shell:       r.options.shell,
			Commands: &jobs.Commands{
				Dispatcher:   r.dispatcher,
				Connection:   c.handle,
				Installation: c.installation,
				Paths:        r.paths,
				Services:     r.services,
				Endpoint:     r.endpoint,
				Home:         r.state.root,
			},
		},
	)
	c.relay = jobs.NewRelay(lifetime)
	c.host = host.New(lifetime, r.options.cwd, r.pipes, t.output)
	c.streams = jobs.NewServiceStreams(lifetime, c.handle, r.pipes, r.services, r.management.draining)
	c.volumes = host.NewVolumes(lifetime, r.options.volumes, t.control)
	defer func() {
		closeErr := errors.Join(c.close(context.Background()), t.close(context.Background()))
		if closeErr == nil {
			return
		}
		// A failed close is reported and retried in place of the connection's
		// ordinary end, a refusal or a lost connection.
		if errors.Is(err, errRegistrationRefused) || errors.Is(err, errConnectionLost) {
			err = closeErr
			return
		}
		err = errors.Join(err, closeErr)
	}()
	c.watchTasks()
	c.watchInstalls()
	return c.run(ctx)
}

// watchTasks joins table completion off the control loop; empty tables wait for admission.
func (c *connection) watchTasks() {
	c.work.Add(1)
	go func() {
		defer c.work.Done()
		for {
			id, ok, err := c.table.Finished(c.ctx)
			if err != nil {
				return
			}
			if ok {
				select {
				case c.finished <- id:
				case <-c.ctx.Done():
					return
				}
				continue
			}
			select {
			case <-c.tasksChanged:
			case <-c.ctx.Done():
				return
			}
		}
	}()
}

func (c *connection) watchInstalls() {
	c.work.Add(1)
	go func() {
		defer c.work.Done()
		for {
			changed, err := c.registration.installs.Changed(c.ctx)
			if err != nil || !changed {
				return
			}
			select {
			case c.installsChanged <- struct{}{}:
			case <-c.ctx.Done():
				return
			}
		}
	}()
}

// launch runs connection-owned IO; state writes use their own noncancelled context.
func (c *connection) launch(run func() connectionWork) {
	c.work.Add(1)
	go func() {
		defer c.work.Done()
		result := run()
		select {
		case c.results <- result:
		case <-c.ctx.Done():
			releaseInstalled(result.installed)
		}
	}()
}

func releaseInstalled(installed *jobs.Installed) {
	if installed != nil {
		for _, lease := range installed.Leases {
			lease.Release()
		}
	}
}

func (c *connection) send(ctx context.Context, message runnerproto.Outbound) error {
	frame, err := runnerproto.Encode(message)
	if err != nil {
		return err
	}
	return c.sendFrame(ctx, frame)
}

func (c *connection) sendFrame(ctx context.Context, frame []byte) error {
	select {
	case c.transport.control <- frame:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	case <-c.transport.done:
		return errors.New("host connection closed")
	}
}

func (c *connection) run(ctx context.Context) error {
	r := c.registration
	if err := c.send(
		c.ctx,
		&runnerproto.Hello{Protocol: runnerproto.Version, DeviceToken: r.token.Load(), Runner: r.options.runner},
	); err != nil {
		return err
	}
	ticker := time.NewTicker(60 * time.Second)
	defer ticker.Stop()
	polled := false
	for {
		if ctx.Err() != nil || r.management.snapshot().Draining && c.table.Len() == 0 {
			return nil
		}
		// Registration precedes backend replies even if both queues are ready.
		select {
		case request := <-c.requests:
			if err := c.request(request); err != nil {
				return err
			}
			continue
		default:
		}
		var drain <-chan struct{}
		if !r.management.snapshot().Draining {
			drain = r.management.draining
		}
		skipPoll, err := c.waitEvent(ctx, ticker.C, drain)
		if err != nil || ctx.Err() != nil {
			return err
		}
		if skipPoll {
			continue
		}
		if !polled && r.management.snapshot().Phase == online {
			polled = true
			if c.table.JobCount() == 0 {
				c.pollVolumes()
			}
		}
	}
}

func (c *connection) pollVolumes() {
	c.volumes.Poll()
	c.launch(func() connectionWork {
		for {
			ok, err := c.volumes.Checked(c.ctx)
			if err != nil {
				return connectionWork{err: err}
			}
			if !ok {
				return connectionWork{}
			}
		}
	})
}

func (c *connection) request(request jobs.Request) error {
	switch r := request.(type) {
	case *jobs.CallRequest:
		c.relay.Call(r)
	case *jobs.ContextRequest:
		r.Register(c.contexts)
	case *jobs.AskRequest:
		frame, err := c.relay.Ask(r)
		if err != nil {
			switch q := r.Question.(type) {
			case *jobs.LocateQuestion:
				q.Answer(nil, err)
			case *jobs.ReserveQuestion:
				q.Answer(0, err)
			}
			return err
		}
		return c.sendFrame(c.ctx, frame)
	}
	return nil
}

func (c *connection) reportInstalls() error {
	installs := c.registration.installs.Current()
	if len(installs) == 0 && !c.installsReported {
		return nil
	}
	c.installsReported = true
	return c.send(c.ctx, &runnerproto.Installs{Installs: installs})
}

func (c *connection) route(message runnerproto.Inbound) error {
	if c.relay.Route(message) {
		return nil
	}
	r := c.registration
	// Authentication handles its own messages; other validated messages route below.
	switch m := any(message).(type) {
	case *runnerproto.HelloOK:
		return c.helloOK(m)
	case *runnerproto.ClaimPending:
		if r.management.snapshot().Phase == online {
			break
		}
		r.management.setPhase(claimPending)
		_, _ = fmt.Fprintln(
			os.Stderr,
			"demi-runner: pairing code: "+m.ClaimToken,
		) // Pairing secret is intentionally console-only.
		slog.Info("waiting to be paired")
		return nil
	case *runnerproto.Claimed:
		if r.management.snapshot().Phase == online {
			break
		}
		r.token.Store(&m.DeviceToken)
		c.launch(func() connectionWork {
			err := r.state.writeToken(context.Background(), m.DeviceToken)
			if err == nil {
				slog.Warn("online")
			}
			return connectionWork{err: err}
		})
		r.management.setPhase(online)
		return nil
	case *runnerproto.HelloError:
		slog.Warn(fmt.Sprintf("registration refused (%s): %s", m.Code, m.Reason))
		if m.Code == runnerproto.HelloErrorCodeAlreadyConnected {
			return errConnectionLost
		}
		r.management.setPhase(rejected)
		return errRegistrationRefused
	case *runnerproto.Ping:
		return c.send(c.ctx, &runnerproto.Pong{Jobs: uint64(c.table.JobCount())})
	}
	if r.management.snapshot().Phase != online {
		return errors.New("backend work arrived before authentication")
	}
	return c.message(message)
}

func (c *connection) close(cleanup context.Context) error {
	c.cancel()
	// All owners see cancellation before any join, so blocking IO releases together.
	err := errors.Join(
		c.table.Close(cleanup),
		c.host.Close(cleanup),
		c.streams.Close(cleanup),
		c.volumes.Close(cleanup),
		c.relay.Close(cleanup),
	)
	c.work.Wait()
	c.directories.Clear(cleanup)
	c.contexts.Close()
	c.installation.Publish(jobs.ManifestAbsent, nil)
	releaseInstalled(c.installed)
	c.registration.management.setJobs(0)
	return errors.Join(err, c.registration.services.StopAll(cleanup))
}

// readJob snapshots kept output before acknowledging and moving bytes to its pipe.
func (c *connection) readJob(request *runnerproto.JobRead) error {
	var stream io.ReadCloser
	var err error
	if reader, ok := c.directories.Output(request.JobID); ok {
		stream, err = reader.Snapshot(c.ctx)
	} else {
		err = errors.New("the job keeps no output: it is unknown or released")
	}
	if stream != nil {
		defer func() { _ = stream.Close() }()
	} // Put owns it too; cover failed acknowledgement.
	reply := &runnerproto.JobReadReply{ID: request.ID}
	if err != nil {
		text := err.Error()
		reply.Error = &text
	}
	if sendErr := c.send(c.ctx, reply); sendErr != nil {
		return sendErr
	}
	if err == nil {
		err = c.registration.pipes.Put(c.ctx, request.Output.URL, stream)
	}
	return process.ReportPipe(c.ctx, c.transport.control, request.Output.ID, err)
}

// pendingRequests drains registrations before a backend reply can consume them.
func (c *connection) pendingRequests() error {
	for {
		select {
		case request := <-c.requests:
			if err := c.request(request); err != nil {
				return err
			}
			continue
		default:
		}
		break
	}
	return nil
}

func (c *connection) completeWork(result connectionWork) error {
	if result.err != nil {
		releaseInstalled(result.installed)
		return result.err
	}
	if result.installed != nil {
		c.installation.Publish(jobs.ManifestReady, result.installed.Manifest)
		releaseInstalled(c.installed)
		c.installed = result.installed
	}
	return nil
}

func (c *connection) helloOK(m *runnerproto.HelloOK) error {
	id := deviceID(m.DeviceID)
	if err := validateDeviceID(id); err != nil {
		return err
	}
	c.launch(func() connectionWork {
		return connectionWork{
			err: c.registration.state.writeConfig(
				context.Background(),
				runnerConfig{BackendURL: c.registration.options.backend, DeviceID: &id},
			),
		}
	})
	c.registration.management.setPhase(online)
	slog.Warn("online")
	return c.reportInstalls()
}

func (c *connection) reportOnlineInstalls() error {
	if c.registration.management.snapshot().Phase != online {
		return nil
	}
	return c.reportInstalls()
}

// waitEvent returns skipPoll for stale installation results, which must bypass the first online poll.
func (c *connection) waitEvent(
	ctx context.Context,
	tick <-chan time.Time,
	drain <-chan struct{},
) (bool, error) {
	r := c.registration
	select {
	case <-ctx.Done():
		return false, nil
	case <-drain:
	case <-c.handle.Done():
		return false, errConnectionLost
	case <-tick:
		if r.management.snapshot().Phase == online && c.table.JobCount() == 0 {
			c.pollVolumes()
		}
	case request := <-c.requests:
		if err := c.request(request); err != nil {
			return false, err
		}
	case id := <-c.finished:
		if id.Kind == jobs.ShellWork {
			c.contexts.Remove(id.ID)
		}
		r.management.setJobs(c.table.JobCount())
	case result := <-c.results:
		if result.installation != 0 && result.installation != c.generation {
			releaseInstalled(result.installed)
			return true, nil
		}
		if err := c.completeWork(result); err != nil {
			return false, err
		}
	case <-c.installsChanged:
		if err := c.reportOnlineInstalls(); err != nil {
			return false, err
		}
	case message, ok := <-c.transport.input:
		if !ok {
			return false, errConnectionLost
		}
		// A registration queued while select chose input still wins over its reply.
		if err := c.pendingRequests(); err != nil {
			return false, err
		}
		if err := c.route(message); err != nil {
			return false, err
		}
	}
	return false, nil
}
