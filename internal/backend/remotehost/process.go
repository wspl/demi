package remotehost

import (
	"context"
	"crypto/rand"
	"sync"

	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerproto"
)

// processFacet starts runner processes independently of shell jobs.
type processFacet struct{ host *Host }

// runnerProcess owns admission and the runner's output and completion publication.
type runnerProcess struct {
	id       string
	link     *Link
	retained bool
	state    *jobState[host.ProcessEnd, host.ProcessOutput]
	lease    *gates.Lease
	finished sync.Once
}

// Spawn starts a process on the runner under Host admission.
func (f processFacet) Spawn(ctx context.Context, request host.SpawnRequest) (*host.StartedProcess, error) {
	var lease *gates.Lease
	var err error
	if !request.Retained {
		lease, err = f.host.admit()
		if err != nil {
			return nil, err
		}
	}
	process := &runnerProcess{
		id:       rand.Text(),
		retained: request.Retained,
		state:    newJobState[host.ProcessEnd, host.ProcessOutput](),
		lease:    lease,
	}
	link, err := f.host.connection()
	if err != nil {
		process.finish(host.ProcessEnd{Kind: host.ProcessLost, Reason: "runner disconnected"})
		return process.handle(), nil
	}
	process.link = link
	link.mu.Lock()
	if link.IsClosed() {
		link.mu.Unlock()
		process.finish(host.ProcessEnd{Kind: host.ProcessLost, Reason: "runner disconnected"})
		return process.handle(), nil
	}
	link.spawns[process.id] = process
	link.mu.Unlock()
	message := &runnerproto.Spawn{SpawnID: process.id, Command: request.Command, CWD: request.CWD}
	if message.CWD == nil {
		message.CWD = new(f.host.cwd)
	}
	if len(request.Args) > 0 {
		message.Args = new(request.Args)
	}
	switch request.Env.Mode {
	case host.Inherit:
	case host.Exactly:
		message.Env = new(request.Env.Values)
	case host.Overlay:
		message.Env = new(request.Env.Values)
		message.InheritEnv = new(true)
	}
	if err := link.send(ctx, message); err != nil {
		link.mu.Lock()
		delete(link.spawns, process.id)
		link.mu.Unlock()
		process.finish(host.ProcessEnd{Kind: host.ProcessLost, Reason: err.Error()})
		return nil, err
	}
	return process.handle(), nil
}

// handle exposes the same process owner as its output, control and wait facets.
func (p *runnerProcess) handle() *host.StartedProcess {
	return &host.StartedProcess{Output: p, Control: p, Wait: p.state.wait}
}

// finish releases process admission once before publishing its end.
func (p *runnerProcess) finish(end host.ProcessEnd) {
	p.finished.Do(func() {
		if p.lease != nil {
			p.lease.Release()
		}
		p.state.finish(end)
	})
}

// Next waits for the next process output chunk.
func (p *runnerProcess) Next(ctx context.Context) (host.ProcessOutput, error) {
	return p.state.next(ctx)
}

// WriteStdin sends bytes to the runner process input.
func (p *runnerProcess) WriteStdin(ctx context.Context, data []byte) error {
	if p.link == nil || p.state.hasEnded() {
		return nil
	}
	return sendStdin(
		ctx,
		p.link,
		data,
		func(chunk []byte) runnerproto.Inbound { return &runnerproto.SpawnStdin{SpawnID: p.id, Bytes: chunk} },
	)
}

// CloseStdin ends the runner process input.
func (p *runnerProcess) CloseStdin(ctx context.Context) error {
	if p.link == nil || p.state.hasEnded() {
		return nil
	}
	return p.link.send(ctx, &runnerproto.SpawnStdinEnd{SpawnID: p.id})
}

// Kill signals the runner process.
func (p *runnerProcess) Kill(ctx context.Context, signal host.Signal) error {
	if p.link == nil || p.state.hasEnded() {
		return nil
	}
	return p.link.send(ctx, &runnerproto.SpawnKill{SpawnID: p.id, Signal: new(runnerproto.Signal(signal))})
}

// Close kills the runner process.
func (p *runnerProcess) Close(ctx context.Context) error { return p.Kill(ctx, host.Kill) }

// processEnd translates the runner's terminal status in its specified precedence.
func processEnd(code *int32, signal *string, spawnError *runnerproto.SpawnError) host.ProcessEnd {
	if code != nil {
		return host.ProcessEnd{Kind: host.ProcessExited, ExitCode: *code}
	}
	if spawnError != nil {
		return host.ProcessEnd{
			Kind:       host.ProcessNotStarted,
			SpawnError: &host.SpawnError{Kind: host.SpawnErrorKind(spawnError.Kind), Detail: spawnError.Detail},
		}
	}
	if signal != nil {
		return host.ProcessEnd{Kind: host.ProcessSignalled, Signal: *signal}
	}
	return host.ProcessEnd{Kind: host.ProcessLost, Reason: "the process ended without a status"}
}
