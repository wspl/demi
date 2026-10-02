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
	"github.com/wspl/demi/internal/runnerwire"
)

type connectionEnd uint8

const (
	stopped connectionEnd = iota
	disconnected
	refused
)

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
	handle           *jobs.ConnectionHandle
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

func (r *registration) serve(ctx context.Context, t *transport) (end connectionEnd, err error) {
	lifetime, cancel := context.WithCancel(ctx)
	c := &connection{registration: r, transport: t, ctx: lifetime, cancel: cancel, installation: &jobs.Installation{}, results: make(chan connectionWork), finished: make(chan jobs.WorkID), tasksChanged: make(chan struct{}, 1), installsChanged: make(chan struct{}, 1)}
	c.handle, c.requests = jobs.NewConnectionHandle(lifetime, t.control)
	c.directories = jobs.OpenDirectories(lifetime, r.options.jobRoot)
	c.contexts = jobs.NewContextTable(r.contexts)
	c.table = jobs.NewTable(lifetime, jobs.Config{Output: t.output, Directories: c.directories, Pipes: r.pipes, Shell: r.options.shell, Commands: &jobs.Commands{Dispatcher: r.dispatcher, Connection: c.handle, Installation: c.installation, Paths: r.paths, Services: r.services, Endpoint: r.endpoint, Home: r.state.root}})
	c.relay = jobs.NewRelay(lifetime)
	c.host = host.New(lifetime, r.options.cwd, r.pipes, t.output)
	c.streams = jobs.NewServiceStreams(lifetime, c.handle, r.pipes, r.services, r.management.draining)
	c.volumes = host.NewVolumes(lifetime, r.options.volumes, t.control)
	defer func() { err = errors.Join(err, c.close(context.Background()), t.close(context.Background())) }()
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
func (c *connection) send(ctx context.Context, message runnerwire.Outbound) error {
	frame, err := runnerwire.Encode(message)
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
func (c *connection) run(ctx context.Context) (connectionEnd, error) {
	r := c.registration
	if err := c.send(c.ctx, &runnerwire.Hello{Protocol: runnerwire.Version, DeviceToken: r.token.Load(), Runner: r.options.runner}); err != nil {
		return disconnected, err
	}
	ticker := time.NewTicker(60 * time.Second)
	defer ticker.Stop()
	polled := false
	for {
		if ctx.Err() != nil || r.management.snapshot().Draining && c.table.Len() == 0 {
			return stopped, nil
		}
		// Registration precedes backend replies even if both queues are ready.
		select {
		case request := <-c.requests:
			if err := c.request(request); err != nil {
				return disconnected, err
			}
			continue
		default:
		}
		var drain <-chan struct{}
		if !r.management.snapshot().Draining {
			drain = r.management.draining
		}
		select {
		case <-ctx.Done():
			return stopped, nil
		case <-drain:
		case <-c.handle.Done():
			return disconnected, nil
		case <-ticker.C:
			if r.management.snapshot().Phase == online && c.table.JobCount() == 0 {
				c.pollVolumes()
			}
		case request := <-c.requests:
			if err := c.request(request); err != nil {
				return disconnected, err
			}
		case id := <-c.finished:
			if id.Kind == jobs.ShellWork {
				c.contexts.Remove(id.ID)
			}
			r.management.setJobs(c.table.JobCount())
		case result := <-c.results:
			if result.installation != 0 && result.installation != c.generation {
				releaseInstalled(result.installed)
				continue
			}
			if result.err != nil {
				releaseInstalled(result.installed)
				return disconnected, result.err
			}
			if result.installed != nil {
				c.installation.Publish(jobs.ManifestReady, result.installed.Manifest)
				releaseInstalled(c.installed)
				c.installed = result.installed
			}
		case <-c.installsChanged:
			if r.management.snapshot().Phase == online {
				if err := c.reportInstalls(); err != nil {
					return disconnected, err
				}
			}
		case message, ok := <-c.transport.input:
			if !ok {
				return disconnected, nil
			}
			// A registration queued while select chose input still wins over its reply.
			for {
				select {
				case request := <-c.requests:
					if err := c.request(request); err != nil {
						return disconnected, err
					}
					continue
				default:
				}
				break
			}
			end, err := c.route(message)
			if end != nil || err != nil {
				if end != nil {
					return *end, err
				}
				return disconnected, err
			}
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
	return c.send(c.ctx, &runnerwire.Installs{Installs: installs})
}
func (c *connection) route(message runnerwire.Inbound) (*connectionEnd, error) {
	if c.relay.Route(message) {
		return nil, nil
	}
	r := c.registration
	// Authentication handles its own messages; other validated messages route below.
	switch m := any(message).(type) {
	case *runnerwire.HelloOK:
		id := deviceID(m.DeviceID)
		if err := validateDeviceID(id); err != nil {
			return nil, err
		}
		c.launch(func() connectionWork {
			return connectionWork{err: r.state.writeConfig(context.Background(), runnerConfig{BackendURL: r.options.backend, DeviceID: &id})}
		})
		r.management.setPhase(online)
		slog.Warn("online")
		return nil, c.reportInstalls()
	case *runnerwire.ClaimPending:
		if r.management.snapshot().Phase == online {
			break
		}
		r.management.setPhase(claimPending)
		_, _ = fmt.Fprintln(os.Stderr, "demi-runner: pairing code: "+m.ClaimToken) // Pairing secret is intentionally console-only.
		slog.Info("waiting to be paired")
		return nil, nil
	case *runnerwire.Claimed:
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
		return nil, nil
	case *runnerwire.HelloError:
		slog.Warn(fmt.Sprintf("registration refused (%s): %s", m.Code, m.Reason))
		end := refused
		if m.Code == runnerwire.HelloErrorCodeAlreadyConnected {
			end = disconnected
		} else {
			r.management.setPhase(rejected)
		}
		return &end, nil
	case *runnerwire.Ping:
		return nil, c.send(c.ctx, &runnerwire.Pong{Jobs: uint64(c.table.JobCount())})
	}
	if r.management.snapshot().Phase != online {
		return nil, errors.New("backend work arrived before authentication")
	}
	return nil, c.message(message)
}

func (c *connection) close(cleanup context.Context) error {
	c.cancel()
	// All owners see cancellation before any join, so blocking IO releases together.
	err := errors.Join(c.table.Close(cleanup), c.host.Close(cleanup), c.streams.Close(cleanup), c.volumes.Close(cleanup), c.relay.Close(cleanup))
	c.work.Wait()
	c.directories.Clear(cleanup)
	c.contexts.Close()
	c.installation.Publish(jobs.ManifestAbsent, nil)
	releaseInstalled(c.installed)
	c.registration.management.setJobs(0)
	return errors.Join(err, c.registration.services.StopAll(cleanup))
}

// readJob snapshots kept output before acknowledging and moving bytes to its pipe.
func (c *connection) readJob(request *runnerwire.JobRead) error {
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
	reply := &runnerwire.JobReadReply{ID: request.ID}
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
