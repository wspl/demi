package remotehost

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"sync"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// relayCall owns one RPC's cancellation, live input demand and relayed pipes.
type relayCall struct {
	jobID, id string
	link      *Link
	ctx       context.Context
	cancel    context.CancelFunc
	mu        sync.Mutex // Protects first stop cause, pipe publication and live-input phase.
	cause     *callStop
	pipes     []*Pipe
	live      livePhase
	requested chan []byte
}
type callStop struct {
	text      string
	cancelled bool
}
type livePhase uint8

const (
	liveIdle livePhase = iota
	liveWaiting
	liveClosed
)

// stop keeps the first stop cause, prioritizing a pipe that has already failed.
func (c *relayCall) stop(reason string, cancelled bool) {
	c.mu.Lock()
	if c.cause != nil {
		c.mu.Unlock()
		return
	}
	for _, pipe := range c.pipes {
		if failure := pipe.Err(); failure != nil {
			reason = failure.Error()
			cancelled = false
			break
		}
	}
	c.cause = &callStop{text: reason, cancelled: cancelled}
	pipes := append([]*Pipe(nil), c.pipes...)
	c.mu.Unlock()
	c.cancel()
	c.endInput()
	for _, pipe := range pipes {
		pipe.Fail(reason)
	}
}

// stopped returns the immutable first terminal cause.
func (c *relayCall) stopped() (callStop, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.cause == nil {
		return callStop{}, false
	}
	return *c.cause, true
}

// liveInput accepts only the single chunk explicitly requested by the handler.
func (c *relayCall) liveInput(data []byte) {
	c.mu.Lock()
	phase := c.live
	next := c.requested
	if phase == liveWaiting {
		c.live = liveIdle
		c.requested = nil
	}
	c.mu.Unlock()
	if phase == liveClosed {
		return
	}
	if phase != liveWaiting || len(data) > runnerwire.StdinChunkBytes {
		c.stop("Unrequested or oversized RPC stdin chunk", false)
		return
	}
	next <- data
}

// endInput releases any outstanding live-input demand at EOF or cancellation.
func (c *relayCall) endInput() {
	c.mu.Lock()
	next := c.requested
	c.requested = nil
	c.live = liveClosed
	c.mu.Unlock()
	if next != nil {
		next <- nil
	}
}

// nextInput requests one live-input chunk; it returns io.EOF once input ended.
func (c *relayCall) nextInput(ctx context.Context) ([]byte, error) {
	c.mu.Lock()
	if c.live == liveClosed {
		c.mu.Unlock()
		return nil, io.EOF
	}
	if c.live == liveWaiting {
		c.mu.Unlock()
		return nil, &host.PortError{Kind: host.PortEnded, Message: "live input is already being read"}
	}
	next := make(chan []byte, 1)
	c.live = liveWaiting
	c.requested = next
	c.mu.Unlock()
	if err := c.link.send(ctx, &runnerwire.RPCStdinPull{CallID: c.id}); err != nil {
		return nil, err
	}
	select {
	case data := <-next:
		if data == nil {
			return nil, io.EOF
		}
		return data, nil
	case <-c.ctx.Done():
		return nil, io.EOF
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// startCall admits a unique RPC ID and transfers its complete lifetime to the link.
func (l *Link) startCall(request *runnerwire.RPCCall) {
	ctx, cancel := context.WithCancel(l.ctx)
	call := &relayCall{jobID: request.JobID, id: request.CallID, link: l, ctx: ctx, cancel: cancel}
	l.mu.Lock()
	_, duplicate := l.calls[request.CallID]
	job := l.jobs[request.JobID]
	if !duplicate {
		l.calls[request.CallID] = call
	}
	l.mu.Unlock()
	if duplicate {
		cancel()
		l.Disconnect("duplicate rpc call " + request.CallID)
		return
	}
	l.task(func() {
		defer cancel()
		code, err := call.run(request, job)
		if err != nil {
			if sendErr := l.send(
				l.ctx,
				&runnerwire.RPCOutput{CallID: call.id, Bytes: []byte(request.Root + ": " + err.Error() + "\n")},
			); sendErr != nil {
				slog.Debug("rpc error not sent: "+sendErr.Error(), "call", call.id)
			}
			code = 1
			if cause, ok := call.stopped(); ok && cause.cancelled {
				code = 130
			}
		}
		l.mu.Lock()
		delete(l.calls, call.id)
		l.mu.Unlock()
		call.stop("rpc call ended", false)
		if err := l.send(l.ctx, &runnerwire.RPCExit{CallID: call.id, ExitCode: code}); err != nil {
			slog.Debug("rpc exit not sent: "+err.Error(), "call", call.id)
		}
	})
}

// run executes policy code and joins it even after a pipe fails, then waits for stdout drain.
func (c *relayCall) run(request *runnerwire.RPCCall, job *Job) (uint8, error) {
	if job == nil {
		return 0, errors.New("rpc requires a live job dispatched to this device")
	}
	if err := c.link.policy.AdmitCall(job.origin); err != nil {
		return 0, err
	}
	stdout := c.link.pipes.ToDevice(c.link.device)
	if err := stdout.HoldSource(); err != nil {
		return 0, err
	}
	var stdin *Pipe
	if request.Stdin {
		stdin = c.link.pipes.FromDevice(c.link.device)
	}
	c.mu.Lock()
	c.pipes = append(c.pipes, stdout)
	if stdin != nil {
		c.pipes = append(c.pipes, stdin)
	}
	c.mu.Unlock()
	if cause, ok := c.stopped(); ok {
		return 0, errors.New(cause.text)
	}
	relayed, err := c.announcePipes(c.ctx, stdout, stdin)
	if err != nil {
		return 0, err
	}
	port := &relayPort{call: c, origin: job.origin, stdout: stdout, stdin: stdin}
	defer port.close()
	invocation := host.RPCInvocation{
		Path:    request.Path,
		Argv:    request.Argv,
		Args:    request.Args,
		JSON:    request.JSON,
		CWD:     request.CWD,
		Env:     request.Env,
		Context: job.origin.Context,
		Caller:  job.origin.Caller,
		Stdin:   request.Stdin,
		Pipes:   relayed,
	}
	code, err := c.dispatch(c.ctx, job.origin, invocation, port)
	if cause, ok := c.stopped(); ok {
		return 0, errors.New(cause.text)
	}
	if err != nil {
		return 0, err
	}
	port.finishStdout()
	if err := stdout.Done(c.ctx); err != nil {
		if cause, ok := c.stopped(); ok {
			return 0, errors.New(cause.text)
		}
		return 0, err
	}
	return code, nil
}

// relayPort serializes each byte direction without locking across pipe IO.
type relayPort struct {
	call            *relayCall
	origin          JobOrigin
	stdout, stdin   *Pipe
	outTurn, inTurn gates.Serial
	writer          *PipeWriter // Protected by outTurn; cleanup runs after the handler joins.
	reader          *PipeReader // Protected by inTurn; cleanup runs after the handler joins.
	outEnded        bool
}

// finishStdout ends a locally held writer, leaving a source handed to a device alone.
func (p *relayPort) finishStdout() {
	if p.outEnded {
		return
	}
	p.outEnded = true
	if p.writer != nil {
		p.writer.End()
		return
	}
	writer, err := p.stdout.Writer()
	if err != nil {
		var pipeErr *PipeError
		if errors.As(err, &pipeErr) && pipeErr.Kind == PipeAlreadyFixed {
			return
		}
		slog.Debug("stdout not ended: "+err.Error(), "call", p.call.id)
		return
	}
	writer.End()
}

// close releases the handler's finite input and any unfinished output.
func (p *relayPort) close() {
	if p.reader != nil {
		if err := p.reader.Close(context.Background()); err != nil {
			slog.Debug("rpc input close failed: "+err.Error(), "call", p.call.id)
		}
	}
	if p.writer != nil {
		p.writer.Fail("the writer went away before the end")
	}
}

// Request handles a command port request while its RPC call lives.
func (p *relayPort) Request(ctx context.Context, request host.PortRequest) (host.PortResponse, error) {
	if p.call.ctx.Err() != nil {
		return nil, &host.PortError{Kind: host.PortEnded, Message: "the call has ended"}
	}
	switch request := request.(type) {
	case *host.PortStdout:
		return p.writeStdout(ctx, request)
	case *host.PortStderr:
		if len(request.Bytes) > 0 {
			if err := p.call.link.send(
				ctx,
				&runnerwire.RPCOutput{CallID: p.call.id, Bytes: runnerwire.WireBytes(request.Bytes)},
			); err != nil {
				return nil, &host.PortError{Kind: host.PortEnded, Message: err.Error(), Err: err}
			}
		}
		return &host.PortWritten{}, nil
	case *host.PortReadStdin:
		return p.readStdin(ctx)
	case *host.PortReadLiveStdin:
		chunk, err := p.call.nextInput(ctx)
		if errors.Is(err, io.EOF) {
			return &host.PortInput{}, nil
		}
		if err != nil {
			return nil, err
		}
		return &host.PortInput{Bytes: new(core.B64Bytes(chunk))}, nil
	case *host.PortStorage:
		reply, err := p.call.link.policy.Storage(p.call.ctx, p.origin, request.Op)
		if err != nil {
			return nil, err
		}
		return &host.PortStored{Reply: reply}, nil
	}
	return nil, &host.PortError{Kind: host.PortFailed, Message: "unknown port request"}
}

// writeStdout serializes access to the RPC output writer.
func (p *relayPort) writeStdout(ctx context.Context, request *host.PortStdout) (host.PortResponse, error) {
	permit, err := p.outTurn.Acquire(ctx)
	if err != nil {
		return nil, err
	}
	defer permit.Release()
	if p.outEnded {
		return nil, &host.PortError{Kind: host.PortEnded, Message: "the call has ended"}
	}
	if p.writer == nil {
		p.writer, err = p.stdout.Writer()
		if err != nil {
			return nil, &host.PortError{Kind: host.PortEnded, Message: "standard output: " + err.Error(), Err: err}
		}
	}
	if err := p.writer.Write(ctx, request.Bytes); err != nil {
		return nil, &host.PortError{Kind: host.PortEnded, Message: err.Error(), Err: err}
	}
	return &host.PortWritten{}, nil
}

// readStdin serializes access to the RPC finite input reader.
func (p *relayPort) readStdin(ctx context.Context) (host.PortResponse, error) {
	permit, err := p.inTurn.Acquire(ctx)
	if err != nil {
		return nil, err
	}
	defer permit.Release()
	if p.stdin == nil {
		return &host.PortInput{}, nil
	}
	if p.reader == nil {
		p.reader, err = p.stdin.Reader()
		if err != nil {
			return nil, &host.PortError{Kind: host.PortEnded, Message: "standard input: " + err.Error(), Err: err}
		}
	}
	chunk, err := p.reader.Next(ctx)
	if errors.Is(err, io.EOF) {
		return &host.PortInput{}, nil
	}
	if err != nil {
		return nil, &host.PortError{Kind: host.PortEnded, Message: err.Error(), Err: err}
	}
	return &host.PortInput{Bytes: new(core.B64Bytes(chunk))}, nil
}

// dispatch watches pipe failures while the policy handles a call, then joins every watcher.
func (c *relayCall) dispatch(
	ctx context.Context,
	origin JobOrigin,
	invocation host.RPCInvocation,
	port *relayPort,
) (uint8, error) {
	watching, cancel := context.WithCancel(ctx)
	var watches sync.WaitGroup
	for _, pipe := range c.pipes {
		watches.Add(1)
		go func() {
			defer watches.Done()
			err := pipe.Failed(watching)
			if errors.Is(err, ErrPipeFailed) {
				c.stop(err.Error(), false)
			} else if !errors.Is(err, context.Canceled) {
				slog.Debug("rpc pipe watch ended: "+err.Error(), "call", c.id)
			}
		}()
	}
	code, err := c.link.policy.Dispatch(ctx, origin, invocation, host.NewRPCPort(port))
	cancel()
	watches.Wait()
	return code, err
}

// announcePipes sends the RPC pipe references before the policy receives their local identities.
func (c *relayCall) announcePipes(ctx context.Context, stdout, stdin *Pipe) (*host.RelayedPipes, error) {
	wire := &runnerwire.RPCPipes{CallID: c.id, Stdout: stdout.WireRef()}
	relayed := &host.RelayedPipes{Stdout: stdout.ID()}
	if stdin != nil {
		wire.Stdin = new(stdin.WireRef())
		relayed.Stdin = new(stdin.ID())
	}
	if err := c.link.send(ctx, wire); err != nil {
		return nil, err
	}
	return relayed, nil
}
