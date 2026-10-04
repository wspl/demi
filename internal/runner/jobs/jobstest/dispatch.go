package jobstest

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// Dispatch supplies a dispatcher, local endpoint and connection owner backed
// by channels. Its cleanup joins workers and releases services and contexts.
type Dispatch struct {
	// Services owns the fixture service registry.
	Services *cmdpkgs.ServiceRegistry
	// Dispatcher runs fixture command invocations.
	Dispatcher *jobs.Dispatcher
	// Server serves the fixture local command endpoint.
	Server *jobs.Server
	// Outgoing carries encoded frames sent to the simulated backend.
	Outgoing <-chan []byte
	t        testing.TB
	manifest *runnerwire.Manifest
	paths    jobs.ContextPaths
	handle   *jobs.Connection
	inbound  chan runnerwire.Inbound
	removals chan removal
	cancel   context.CancelFunc
	done     chan struct{}
	once     sync.Once
	closed   chan struct{}
	err      error
	leases   []*cmdpkgs.ServiceLease
}
type removal struct {
	job  string
	done chan struct{}
}

// NewDispatch installs manifest in root and uses the test executable for aliases.
// It registers cleanup with t, including when construction fails. Pipes remains
// caller-owned and must outlive the fixture.
func NewDispatch(
	ctx context.Context,
	t testing.TB,
	root string,
	manifest json.RawMessage,
	pipes *process.PipeClient,
) *Dispatch {
	t.Helper()
	lifetime, cancel := context.WithCancel(ctx)
	d := &Dispatch{t: t, cancel: cancel, closed: make(chan struct{})}
	t.Cleanup(func() {
		if err := d.Close(context.Background()); err != nil {
			t.Errorf("dispatch cleanup: %v", err)
		}
	})
	var err error
	d.Services, err = cmdpkgs.NewServiceRegistry(
		lifetime,
		filepath.Join(root, "artifacts"),
		"",
		root,
		map[string]string{},
	)
	if err != nil {
		t.Fatal(err)
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	d.paths, err = jobs.NewContextPaths(lifetime, filepath.Join(root, "manifests"), executable)
	if err != nil {
		t.Fatal(err)
	}
	installed, err := jobs.Install(lifetime, manifest, d.paths, d.Services, map[string]struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	d.manifest, d.leases = installed.Manifest, installed.Leases
	contexts := &jobs.Contexts{}
	d.Dispatcher = &jobs.Dispatcher{Contexts: contexts, Services: d.Services, Pipes: pipes}
	d.Server, err = jobs.StartServer(lifetime, d.Dispatcher)
	if err != nil {
		t.Fatal(err)
	}
	control := make(chan []byte, 32)
	d.Outgoing = control
	handle, requests := jobs.NewConnection(lifetime, control)
	d.handle = handle
	d.inbound = make(chan runnerwire.Inbound, 32)
	d.removals = make(chan removal, 16)
	d.done = make(chan struct{})
	go d.serve(lifetime, requests, contexts, control)
	return d
}

// Context creates a live job context and its registration. The fixture also registers its cleanup,
// so a failed test cannot leave a live registration.
func (d *Dispatch) Context(
	ctx context.Context,
	jobID string,
	command commandwire.Context,
) (*jobs.ExecutionContext, *ContextRegistration, error) {
	edits := commandwire.EditContext{
		Directory: filepath.Join(d.paths.Directory, jobID),
		Lock:      filepath.Join(d.paths.Directory, "edits.lock"),
	}
	execution, err := jobs.NewExecutionContext(ctx, jobID, command, d.manifest, edits, d.handle, d.paths)
	if err != nil {
		return nil, nil, err
	}
	leases, err := jobs.Leases(ctx, d.manifest, d.Services)
	if err == nil {
		err = d.handle.RegisterContext(ctx, execution, leases)
	}
	if err != nil {
		for _, lease := range leases {
			lease.Release()
		}
		_ = execution.Close(context.WithoutCancel(ctx))
		return nil, nil, err
	}
	registration := &ContextRegistration{dispatch: d, execution: execution}
	d.t.Cleanup(func() {
		if err := registration.Close(context.Background()); err != nil {
			d.t.Errorf("context cleanup: %v", err)
		}
	})
	return execution, registration, nil
}

// Deliver hands a validated backend message to the channel-backed owner.
func (d *Dispatch) Deliver(ctx context.Context, message runnerwire.Inbound) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-d.done:
		return context.Canceled
	case d.inbound <- message:
		return nil
	}
}

// Close stops the endpoint, revokes contexts and joins the connection and services.
// It is idempotent and may be called before the registered test cleanup.
func (d *Dispatch) Close(ctx context.Context) error {
	d.once.Do(func() {
		go func() {
			d.cancel()
			if d.Server != nil {
				d.err = errors.Join(d.err, d.Server.Close(context.Background()))
			}
			if d.done != nil {
				<-d.done
			}
			for _, lease := range d.leases {
				lease.Release()
			}
			if d.Services != nil {
				d.err = errors.Join(d.err, d.Services.Close(context.Background()))
			}
			close(d.closed)
		}()
	})
	select {
	case <-d.closed:
		return d.err
	case <-ctx.Done():
		return ctx.Err()
	}
}

// ContextRegistration owns one fixture context registration and its alias directory.
type ContextRegistration struct {
	dispatch  *Dispatch
	execution *jobs.ExecutionContext
}

// Close revokes the context and waits for its removal, then removes aliases.
// Callers first join invocations using it. Repeated calls are harmless.
func (r *ContextRegistration) Close(ctx context.Context) error {
	r.execution.Cancel()
	removal := removal{job: r.execution.JobID, done: make(chan struct{})}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-r.dispatch.done:
	case r.dispatch.removals <- removal:
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-r.dispatch.done:
		case <-removal.done:
		}
	}
	return r.execution.Close(ctx)
}

// serve gives requests priority over backend answers, matching the connection owner.
func (d *Dispatch) serve(
	ctx context.Context,
	requests <-chan jobs.Request,
	contexts *jobs.Contexts,
	control chan<- []byte,
) {
	defer close(d.done)
	relay := jobs.NewRelay(ctx)
	defer func() { _ = relay.Close(context.Background()) }()
	table := jobs.NewContextTable(contexts)
	defer table.Close()
	handle := func(request jobs.Request) { handleRequest(ctx, request, relay, table, control) }
	for {
		select {
		case <-ctx.Done():
			return
		case <-d.handle.Done():
			return
		default:
		}
		select {
		case request := <-requests:
			handle(request)
			continue
		default:
		}
		select {
		case <-ctx.Done():
			return
		case <-d.handle.Done():
			return
		case request := <-requests:
			handle(request)
		case message := <-d.inbound:
			// An already queued registration must precede even an immediately queued reply.
			for {
				select {
				case request := <-requests:
					handle(request)
				default:
					goto routed
				}
			}
		routed:
			relay.Route(message)
		case removal := <-d.removals:
			table.Remove(removal.job)
			close(removal.done)
		}
	}
}

func handleRequest(
	ctx context.Context,
	request jobs.Request,
	relay *jobs.Relay,
	table *jobs.ContextTable,
	control chan<- []byte,
) {
	switch request := request.(type) {
	case *jobs.CallRequest:
		relay.Call(request)
	case *jobs.ContextRequest:
		request.Register(table)
	case *jobs.AskRequest:
		frame, err := relay.Ask(request)
		if err != nil {
			switch q := request.Question.(type) {
			case *jobs.LocateQuestion:
				q.Answer(nil, err)
			case *jobs.ReserveQuestion:
				q.Answer(0, err)
			}
			return
		}
		select {
		case <-ctx.Done():
		case control <- frame:
		}
	}
}
