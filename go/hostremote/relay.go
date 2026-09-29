package hostremote

import (
	"context"
	"errors"
	"io"
	"sync"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/gates"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

type callEntry struct {
	jobID      string
	ctx        context.Context
	cancel     context.CancelFunc
	mu         sync.Mutex
	cause      string
	cancelled  bool
	live       chan []byte
	liveClosed bool
	pipes      []*Pipe
}

func (c *callEntry) stop(reason string, cancelled bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.cause != "" {
		return
	}
	for _, pipe := range c.pipes {
		if failure := pipe.Failure(); failure != nil {
			reason = failure.Error()
			cancelled = false
			break
		}
	}
	c.cause, c.cancelled = reason, cancelled
	c.cancel()
	if c.live != nil {
		c.live <- nil
		c.live = nil
	}
	c.liveClosed = true
	for _, pipe := range c.pipes {
		pipe.Fail(reason)
	}
}
func (c *callEntry) stopped() (string, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.cause, c.cancelled
}
func (c *callEntry) liveInput(bytes []byte) {
	c.mu.Lock()
	if c.liveClosed {
		c.mu.Unlock()
		return
	}
	next := c.live
	c.live = nil
	c.mu.Unlock()
	if next == nil || len(bytes) > runnerproto.StdinChunkBytes {
		c.stop("Unrequested or oversized RPC stdin chunk", false)
		return
	}
	next <- bytes
}
func (c *callEntry) endLiveInput() {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.live != nil {
		c.live <- nil
		c.live = nil
	}
	c.liveClosed = true
}
func (c *callEntry) nextLive(ctx context.Context, l *Link, id string) ([]byte, error) {
	c.mu.Lock()
	if c.liveClosed {
		c.mu.Unlock()
		return nil, nil
	}
	if c.live != nil {
		c.mu.Unlock()
		return nil, errors.New("live input is already being read")
	}
	next := make(chan []byte, 1)
	c.live = next
	c.mu.Unlock()
	if err := l.send(ctx, runnerproto.InboundRPCStdinPull{CallID: id}); err != nil {
		return nil, err
	}
	select {
	case bytes := <-next:
		return bytes, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-c.ctx.Done():
		return nil, c.ctx.Err()
	}
}
func (l *Link) startRPC(call runnerproto.OutboundRPCCall) {
	ctx, cancel := context.WithCancel(l.ctx)
	entry := &callEntry{jobID: call.JobID, ctx: ctx, cancel: cancel}
	l.mu.Lock()
	_, duplicate := l.calls[call.CallID]
	job := l.jobs[call.JobID]
	if !duplicate {
		l.calls[call.CallID] = entry
	}
	l.mu.Unlock()
	if duplicate {
		cancel()
		l.Disconnect("duplicate rpc call " + call.CallID)
		return
	}
	l.spawn(func() {
		defer cancel()
		code, err := l.runRPC(call, job, entry)
		if err != nil {
			// The connection may already be gone; there is then nobody to answer.
			_ = l.send(l.ctx, runnerproto.InboundRPCOutput{CallID: call.CallID, Bytes: []byte(call.Root + ": " + err.Error() + "\n")})
			code = 1
			if _, cancelled := entry.stopped(); cancelled {
				code = 130
			}
		}
		l.mu.Lock()
		delete(l.calls, call.CallID)
		l.mu.Unlock()
		entry.stop("rpc call ended", false)
		_ = l.send(l.ctx, runnerproto.InboundRPCExit{CallID: call.CallID, ExitCode: code})
	})
}
func (l *Link) runRPC(call runnerproto.OutboundRPCCall, job *jobEntry, entry *callEntry) (uint8, error) {
	if job == nil {
		return 0, errors.New("rpc requires a live job dispatched to this device")
	}
	if err := l.policy.AdmitCall(job.origin); err != nil {
		return 0, err
	}
	stdout := l.pipes.ToDevice(l.device)
	if err := stdout.HoldSource(); err != nil {
		return 0, err
	}
	var stdin *Pipe
	var stdinRef *runnerproto.PipeRef
	var stdinID *string
	if call.Stdin {
		stdin = l.pipes.FromDevice(l.device)
		ref := stdin.WireRef()
		stdinRef = &ref
		stdinID = new(stdin.ID())
	}
	entry.mu.Lock()
	entry.pipes = append(entry.pipes, stdout)
	if stdin != nil {
		entry.pipes = append(entry.pipes, stdin)
	}
	cause := entry.cause
	entry.mu.Unlock()
	if cause != "" {
		stdout.Fail(cause)
		if stdin != nil {
			stdin.Fail(cause)
		}
		return 0, errors.New(cause)
	}
	if err := l.send(entry.ctx, runnerproto.InboundRPCPipes{CallID: call.CallID, Stdin: stdinRef, Stdout: stdout.WireRef()}); err != nil {
		return 0, err
	}
	invocation := shell.RPCInvocation{Path: call.Path, Argv: call.Argv, Args: call.Args, JSON: call.JSON, Cwd: call.Cwd, Env: call.Env, Context: job.origin.Context, Caller: job.origin.Caller, Stdin: call.Stdin, Pipes: &shell.RelayedPipes{Stdin: stdinID, Stdout: stdout.ID()}}
	port := &relayPort{link: l, id: call.CallID, entry: entry, origin: job.origin, stdout: stdout, stdin: stdin, outTurn: gates.NewSerialGate(), inTurn: gates.NewSerialGate()}
	// Pipe monitors belong to this invocation and are joined even if the handler
	// returns immediately. Cancellation never abandons a committing handler.
	monitorCtx, stopMonitors := context.WithCancel(entry.ctx)
	var monitors sync.WaitGroup
	for _, pipe := range entry.pipes {
		monitors.Add(1)
		go func() {
			defer monitors.Done()
			if err := pipe.Done(monitorCtx); err != nil && monitorCtx.Err() == nil {
				entry.stop(err.Error(), false)
			}
		}()
	}
	code, err := l.policy.Dispatch(entry.ctx, job.origin, invocation, shell.RPCPort{Transport: port, Context: entry.ctx})
	if cause, _ := entry.stopped(); cause != "" {
		err = errors.New(cause)
	}
	if err == nil {
		err = port.finishStdout()
		if err == nil {
			err = stdout.Done(entry.ctx)
		}
	}
	stopMonitors()
	monitors.Wait()
	if cause, _ := entry.stopped(); cause != "" {
		return 0, errors.New(cause)
	}
	return code, err
}

type relayPort struct {
	link            *Link
	id              string
	entry           *callEntry
	origin          JobOrigin
	stdout, stdin   *Pipe
	writer          *PipeWriter
	reader          *PipeReader
	outTurn, inTurn *gates.SerialGate
	finished        bool
}

func (p *relayPort) finishStdout() error {
	permit, err := p.outTurn.Acquire(p.entry.ctx)
	if err != nil {
		return err
	}
	defer permit.Release()
	p.finished = true
	if p.writer == nil {
		p.stdout.owner.mu.Lock()
		handed := p.stdout.source.device != ""
		p.stdout.owner.mu.Unlock()
		if handed {
			return nil
		}
		p.writer, err = p.stdout.Writer()
		if err != nil {
			return err
		}
	}
	return p.writer.Close()
}
func (p *relayPort) Request(ctx context.Context, request shell.PortRequest) (shell.PortResponse, error) {
	var err error
	switch request := request.(type) {
	case shell.PortStdout:
		permit, failure := p.outTurn.Acquire(ctx)
		if failure != nil {
			return nil, failure
		}
		defer permit.Release()
		if p.finished {
			return nil, &shell.PortError{Kind: shell.PortEnded, Message: "the call has ended"}
		}
		if p.writer == nil {
			p.writer, err = p.stdout.Writer()
			if err != nil {
				return nil, &shell.PortError{Kind: shell.PortEnded, Message: "standard output: " + err.Error()}
			}
		}
		_, err = p.writer.WriteContext(ctx, request.Bytes.Bytes())
	case shell.PortStderr:
		if bytes := request.Bytes.Bytes(); len(bytes) > 0 {
			err = p.link.send(ctx, runnerproto.InboundRPCOutput{CallID: p.id, Bytes: bytes})
		}
	case shell.PortReadStdin:
		permit, failure := p.inTurn.Acquire(ctx)
		if failure != nil {
			return nil, failure
		}
		defer permit.Release()
		if p.stdin == nil {
			return shell.PortInput{}, nil
		}
		if p.reader == nil {
			p.reader, err = p.stdin.Reader()
			if err != nil {
				return nil, &shell.PortError{Kind: shell.PortEnded, Message: "standard input: " + err.Error()}
			}
		}
		bytes, failure := p.reader.Next(ctx)
		if failure == io.EOF {
			return shell.PortInput{}, nil
		}
		if failure != nil {
			return nil, &shell.PortError{Kind: shell.PortEnded, Message: failure.Error()}
		}
		value := core.NewB64Bytes(bytes)
		return shell.PortInput{Bytes: &value}, nil
	case shell.PortReadLiveStdin:
		bytes, failure := p.entry.nextLive(ctx, p.link, p.id)
		if failure != nil {
			return nil, &shell.PortError{Kind: shell.PortEnded, Message: failure.Error()}
		}
		if bytes == nil {
			return shell.PortInput{}, nil
		}
		value := core.NewB64Bytes(bytes)
		return shell.PortInput{Bytes: &value}, nil
	case shell.PortStorageRequest:
		reply, err := p.link.policy.Storage(p.entry.ctx, p.origin, request.Op)
		return shell.PortStorageResponse{Reply: reply}, err
	}
	if err != nil {
		return nil, &shell.PortError{Kind: shell.PortEnded, Message: err.Error()}
	}
	return shell.PortWritten{}, nil
}
