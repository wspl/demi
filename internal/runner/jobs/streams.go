package jobs

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// ServiceStreams owns user calls on resident services, their pipe IO and the
// connection's last package bindings. Its owner must Close it before services stop.
type ServiceStreams struct {
	connection *ConnectionHandle
	pipes      *process.PipeClient
	services   *cmdpkgs.ServiceHandle
	lifetime   context.Context
	cancel     context.CancelFunc
	draining   <-chan struct{}
	// mu protects stream admission and bindings, never service waits.
	mu       sync.Mutex
	bindings map[string]*streamBinding
	closing  bool
	workers  sync.WaitGroup
	once     sync.Once
	done     chan struct{}
}
type streamBinding struct {
	digest string
	lease  *cmdpkgs.ServiceLease
}

// NewServiceStreams creates an owner that cancels streams with ctx. Closing
// draining refuses new calls while retaining the service binding, as Rust does.
func NewServiceStreams(ctx context.Context, connection *ConnectionHandle, pipes *process.PipeClient, services *cmdpkgs.ServiceHandle, draining <-chan struct{}) *ServiceStreams {
	lifetime, cancel := context.WithCancel(ctx)
	return &ServiceStreams{connection: connection, pipes: pipes, services: services, lifetime: lifetime, cancel: cancel, draining: draining, bindings: make(map[string]*streamBinding), done: make(chan struct{})}
}

// HandleOpen registers a service_open request and starts its owned work without
// waiting for completion. It refuses other messages and a closed connection.
func (s *ServiceStreams) HandleOpen(message runnerwire.Inbound) error {
	request, ok := message.(*runnerwire.ServiceOpen)
	if !ok {
		return errors.New("not a service_open request")
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		return err
	}
	artifact, hasArtifact := request.Package.Targets[string(target)]
	var moved *streamBinding
	var oldLease *cmdpkgs.ServiceLease
	s.mu.Lock()
	if s.closing || s.lifetime.Err() != nil {
		s.mu.Unlock()
		return errors.New("host connection closed")
	}
	if hasArtifact {
		old := s.bindings[request.Package.ID]
		if old == nil || old.digest != artifact.SHA256 {
			if old != nil {
				oldLease = old.lease
			}
			moved = &streamBinding{digest: artifact.SHA256}
			s.bindings[request.Package.ID] = moved
		}
	}
	s.workers.Add(1)
	s.mu.Unlock()
	if oldLease != nil {
		oldLease.Release()
	}
	go func() { defer s.workers.Done(); s.run(request, moved) }()
	return nil
}

// Close cancels and joins every stream, then releases its service bindings.
// If ctx ends first, a later Close can join shutdown. It is idempotent.
func (s *ServiceStreams) Close(ctx context.Context) error {
	s.once.Do(func() {
		s.mu.Lock()
		s.closing = true
		s.mu.Unlock()
		s.cancel()
		go func() {
			s.workers.Wait()
			s.mu.Lock()
			bindings := s.bindings
			s.bindings = nil
			s.mu.Unlock()
			for _, binding := range bindings {
				if binding.lease != nil {
					binding.lease.Release()
				}
			}
			close(s.done)
		}()
	})
	select {
	case <-s.done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// run owns a service invocation, its binding lease and both pipe directions.
func (s *ServiceStreams) run(request *runnerwire.ServiceOpen, moved *streamBinding) {
	ctx, cancel := context.WithCancel(s.lifetime)
	defer cancel()
	log := func(text string) {
		slog.Info("stream:"+request.Operation+" "+text, "conversation", request.Context.Conversation)
	}
	refuse := func(code runnerwire.ServiceErrorCode, message string) {
		log("refused (" + string(code) + "): " + message)
		if err := s.connection.send(ctx, &runnerwire.ServiceError{StreamID: request.StreamID, Code: code, Message: message}); err != nil && ctx.Err() == nil {
			slog.Warn("service_error encoding failed", "error", err)
		}
	}
	if moved != nil {
		lease, err := s.services.Lease(ctx, moved.digest)
		if err != nil {
			refuse(runnerwire.ServiceErrorCodeServiceFailed, err.Error())
			return
		}
		s.mu.Lock()
		held := s.bindings[request.Package.ID] == moved && !s.closing
		if held {
			moved.lease = lease
		}
		s.mu.Unlock()
		if !held {
			lease.Release()
		}
	}
	select {
	case <-s.draining:
		refuse(runnerwire.ServiceErrorCodeRefused, "the runner is draining for an upgrade")
		return
	default:
	}
	if !slices.Contains(request.Package.Operations, request.Operation) {
		refuse(runnerwire.ServiceErrorCodeUnknownOperation, fmt.Sprintf("%s has no operation %s", request.Package.ID, request.Operation))
		return
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		refuse(runnerwire.ServiceErrorCodeServiceFailed, err.Error())
		return
	}
	if artifact, ok := request.Package.Targets[string(target)]; ok {
		lease, err := s.services.Lease(ctx, artifact.SHA256)
		if err != nil {
			refuse(runnerwire.ServiceErrorCodeServiceFailed, err.Error())
			return
		}
		defer lease.Release()
	}
	resolver := &streamArtifacts{connection: s.connection, stream: request.StreamID}
	registration := s.services.Invoking(request.StreamID, request.Package.ID, resolver)
	defer registration.Release()
	resident, err := s.services.Acquire(ctx, request.Package, resolver, s.connection)
	if err != nil {
		refuse(runnerwire.ServiceErrorCodeServiceFailed, err.Error())
		return
	}
	args := json.RawMessage(`{}`)
	if request.Args != nil {
		args = *request.Args
	}
	invocation := commandwire.Invocation{Operation: request.Operation, InvocationID: request.StreamID, Context: request.Context, Args: args, Cwd: request.CWD, Env: map[string]string{}, JSON: request.JSON}
	input, response, err := resident.Client().Invoke(ctx, invocation)
	if err != nil {
		refuse(runnerwire.ServiceErrorCodeServiceFailed, resident.Failure(ctx, err).Error())
		return
	}
	defer input.Cancel()
	if err = s.connection.send(ctx, &runnerwire.ServiceOpened{StreamID: request.StreamID}); err != nil {
		return
	}
	log("opened")
	source := &streamInput{pipes: s.pipes, url: request.Input.URL}
	defer source.close()
	uploads := make(chan []byte, 4)
	uploadCtx, stopUpload := context.WithCancel(ctx)
	defer stopUpload()
	uploaded := make(chan error, 1)
	go func() {
		err := s.pipes.Put(uploadCtx, request.Output.URL, newInvocationBody(uploadCtx, cmdsdk.NewInput(&chunkSource{chunks: uploads})))
		if err != nil {
			cancel()
		}
		uploaded <- err
	}()
	sink := &streamOutput{uploads: uploads, tail: process.NewTail(runnerwire.ServiceStderrChars), conversation: request.Context.Conversation, operation: request.Operation}
	completion, result := (cmdsdk.Exchange{Input: input, Output: response}).Run(ctx, source, sink)
	if result != nil {
		stopUpload()
	}
	close(uploads)
	upload := <-uploaded
	if line := sink.lines.Finish(); line != nil {
		sink.log(*line)
	}
	var inputResult error
	if result != nil {
		var exchange *cmdsdk.ExchangeError
		if errors.As(result, &exchange) && exchange.Side == "input" {
			inputResult = exchange.Cause
		} else if errors.As(result, &exchange) && exchange.Side == "service" && !errors.Is(result, context.Canceled) {
			log("failed: " + result.Error())
			inputResult = errors.New("the service stream's invocation failed")
		} else {
			inputResult = errors.New("service stream cancelled")
		}
	}
	if upload != nil || inputResult != nil {
		cancel()
	}
	if err := process.ReportPipe(s.lifetime, s.connection.Control, request.Input.ID, inputResult); err != nil && s.lifetime.Err() == nil {
		slog.Warn("pipe reporting failed", "error", err)
	}
	if err := process.ReportPipe(s.lifetime, s.connection.Control, request.Output.ID, upload); err != nil && s.lifetime.Err() == nil {
		slog.Warn("pipe reporting failed", "error", err)
	}
	if result == nil {
		if completion.Error != nil {
			log("failed: " + completion.Error.Code + ": " + completion.Error.Message)
		}
		if err := s.connection.send(s.lifetime, &runnerwire.ServiceDone{StreamID: request.StreamID, ExitCode: completion.ExitCode, Stderr: sink.tail.Text()}); err != nil && s.lifetime.Err() == nil {
			slog.Warn("service_done encoding failed", "error", err)
		}
	}
	log("ended")
}

// streamInput opens a user input pipe only after the service's first pull.
type streamInput struct {
	pipes *process.PipeClient
	url   string
	body  io.ReadCloser
}

func (s *streamInput) Next(ctx context.Context) ([]byte, error) {
	if s.body == nil {
		body, err := s.pipes.Open(ctx, s.url)
		if err != nil {
			return nil, err
		}
		s.body = body
	}
	bytes := make([]byte, commandwire.MaxRecordBytes)
	n, err := s.body.Read(bytes)
	if n > 0 {
		return bytes[:n], nil
	}
	return nil, err
}
func (s *streamInput) close() {
	if s.body != nil {
		_ = s.body.Close()
	}
} // The exchange already reports read errors.

// streamOutput backpressures stdout and logs bounded stderr with its conversation.
type streamOutput struct {
	uploads                 chan<- []byte
	tail                    *process.Tail
	lines                   process.LineSplitter
	conversation, operation string
}

func (s *streamOutput) Stdout(ctx context.Context, bytes []byte) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case s.uploads <- append([]byte{}, bytes...):
		return nil
	}
}
func (s *streamOutput) Stderr(_ context.Context, bytes []byte) error {
	s.tail.Push(bytes)
	for _, line := range s.lines.Push(bytes) {
		s.log(line)
	}
	return nil
}
func (s *streamOutput) log(line string) {
	slog.Info(line, "source", "stream:"+s.operation, "conversation", s.conversation)
}
