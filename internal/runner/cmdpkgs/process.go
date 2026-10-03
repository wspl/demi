package cmdpkgs

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"os"
	"os/exec"
	"sync"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

func (r *ServiceRegistry) live(
	life *serviceLife,
	descriptor commandwire.PackageDescriptor,
	artifact commandwire.PackageArtifact,
	resolver ArtifactResolver,
	numbers NumberSource,
) {
	defer r.work.Done()
	defer life.cancel()
	hold := r.cache.Holds().Hold(artifact.SHA256)
	defer hold.Release()
	executable, err := r.cache.Install(
		life.stop,
		Wanted{
			Package:  descriptor.ID,
			Name:     "program",
			Version:  descriptor.Version,
			Artifact: artifact,
			Form:     &commandwire.ArtifactFile{},
		},
		resolver,
	)
	if err == nil {
		err = r.runService(life, descriptor, executable, numbers)
	}
	select {
	case <-life.ready:
		var exit *ServiceExit
		if errors.As(err, &exit) {
			slog.Warn("service " + descriptor.ID + " " + exit.Reason.String())
		} else {
			slog.Info("service " + descriptor.ID + " stopped")
		}
	default:
		var failure *RuntimeError
		if !errors.Is(err, context.Canceled) && (!errors.As(err, &failure) || failure.Kind != Cancelled) {
			slog.Warn("service " + descriptor.ID + " did not start: " + err.Error())
		}
	}
	r.mu.Lock()
	life.err = runtimeFailure(err)
	select {
	case <-life.ready:
	default:
		close(life.ready)
	}
	if entry := r.entries[artifact.SHA256]; entry != nil && entry.current == life {
		entry.current = nil
		if entry.leases == 0 {
			delete(r.entries, artifact.SHA256)
		}
	}
	close(life.done)
	r.mu.Unlock()
}

// runService owns every child pipe, the HTTP/2 client, stream responders and reaper.
func (r *ServiceRegistry) runService(
	life *serviceLife,
	descriptor commandwire.PackageDescriptor,
	executable string,
	numbers NumberSource,
) (result error) {
	cmd := r.serviceCommand(executable)
	// Caller-owned pipes let the stderr drain finish after Wait reaps the child.
	var pipes []*os.File
	defer func() {
		for _, pipe := range pipes {
			_ = pipe.Close()
		}
	}() // Already-closed ends are harmless; the result describes the service.
	input, output, diagnostic, err := servicePipes(cmd, &pipes)
	if err != nil {
		return err
	}
	child := process.Wrap(cmd, true, process.ChildAttributes{})
	childctx, cancelChild := context.WithCancel(context.Background())
	defer cancelChild()
	if err := startServiceChild(childctx, life.stop, child, cancelChild); err != nil {
		return err
	}
	for _, pipe := range []*os.File{pipes[0], pipes[3], pipes[5]} {
		_ = pipe.Close()
	} // Parent no longer owns the child ends.
	return r.serveService(childctx, life, descriptor, numbers, child, input, output, diagnostic)
}

type watchedConnection struct {
	net.Conn
	once  sync.Once
	ended chan struct{}
	err   error
}

func (c *watchedConnection) failed(err error) {
	if err != nil {
		c.once.Do(func() {
			c.err = err
			close(c.ended)
		})
	}
}

func (c *watchedConnection) Read(b []byte) (int, error) {
	n, err := c.Conn.Read(b)
	c.failed(err)
	return n, err
}

func (c *watchedConnection) Write(b []byte) (int, error) {
	n, err := c.Conn.Write(b)
	c.failed(err)
	return n, err
}

func drainStderr(pipe *os.File, service string, tail *process.Tail, done chan<- struct{}) {
	defer close(done)
	var lines process.LineSplitter
	buffer := make([]byte, 4096)
	for {
		n, err := pipe.Read(buffer)
		if n > 0 {
			tail.Push(buffer[:n])
			for _, line := range lines.Push(buffer[:n]) {
				slog.Info(line, "source", "service:"+service)
			}
		}
		if err != nil {
			if !errors.Is(err, io.EOF) && !errors.Is(err, os.ErrClosed) {
				slog.Warn("service " + service + " standard error: " + err.Error())
			}
			break
		}
	}
	if line, ok := lines.Finish(); ok {
		slog.Info(line, "source", "service:"+service)
	}
}

func connectionLost(err error) bool {
	var op *net.OpError
	var path *os.PathError
	return errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) || errors.Is(err, net.ErrClosed) ||
		errors.Is(err, os.ErrClosed) ||
		errors.As(err, &op) ||
		errors.As(err, &path)
}

// shutdownService asks a connected service to release its resources, giving the
// request and process exit one shared six-second deadline.
// The caller keeps draining stderr, then kills and reaps if this returns false.
func shutdownService(ctx context.Context, client *cmdsdk.Client, exited <-chan struct{}, source string) bool {
	ctx, cancel := context.WithTimeout(ctx, 6*time.Second)
	defer cancel()
	requested := make(chan struct{})
	go func() {
		// A failed or answered request does not establish that the process exited.
		// Keep the connection alive and wait for exit or the shared deadline.
		_ = client.Shutdown(ctx)
		close(requested)
	}()
	defer func() {
		cancel()
		<-requested
	}()
	select {
	case <-exited:
		return true
	case <-ctx.Done():
		slog.Warn(source + " did not shut down within 6 seconds and was killed")
		return false
	}
}

func connectService(
	startctx context.Context,
	cancelStart context.CancelCauseFunc,
	life *serviceLife,
	descriptor commandwire.PackageDescriptor,
	connection *watchedConnection,
) (
	client *cmdsdk.Client,
	info commandwire.ServiceInfo,
	numberStream, artifactStream *cmdsdk.RequestStream,
	startErr, err error,
) {
	timedOut := make(chan struct{})
	startTimer := time.AfterFunc(10*time.Second, func() {
		cancelStart(context.DeadlineExceeded)
		close(timedOut)
	})
	interrupted := make(chan struct{})
	stopStarting := context.AfterFunc(life.stop, func() {
		cancelStart(context.Canceled)
		close(interrupted)
	})
	client, err = cmdsdk.Connect(startctx, connection)
	if err == nil {
		info, err = client.Info(startctx)
	}
	if err == nil && !descriptor.Serves(info) {
		err = &RuntimeError{Kind: CatalogMismatch}
	}
	if err == nil {
		numberStream, err = client.Numbers(startctx)
	}
	if err == nil {
		artifactStream, err = client.Artifacts(startctx)
	}
	if !startTimer.Stop() {
		<-timedOut
	}
	if !stopStarting() {
		<-interrupted
	}
	startErr = context.Cause(startctx)
	if err == nil {
		err = startErr
	}
	return
}

func failedServiceStart(
	life *serviceLife,
	service string,
	client *cmdsdk.Client,
	exited <-chan struct{},
	reaped *bool,
	startErr, err error,
) error {
	// A connected startup has the same shutdown obligations as a ready
	// service: it may already hold resources despite a bad catalog or
	// an unanswered startup request. Keep its connection and stderr alive
	// through the shutdown deadline; the finalizer kills only afterward.
	exitedBeforeStop := false
	select {
	case <-exited:
		*reaped = true
		exitedBeforeStop = true
	default:
	}
	if client != nil && !*reaped {
		*reaped = shutdownService(context.Background(), client, exited, "service:"+service)
	}
	if life.stop.Err() != nil {
		return &RuntimeError{Kind: Cancelled, Cause: life.stop.Err()}
	}
	if exitedBeforeStop {
		return &ServiceExit{Service: service, Reason: ExitReason{Kind: ProcessExited}}
	}
	var runtimeErr *RuntimeError
	if errors.As(err, &runtimeErr) {
		return err
	}
	reason := ExitReason{Kind: ProtocolBroken, Detail: err.Error()}
	if errors.Is(startErr, context.DeadlineExceeded) {
		reason = ExitReason{Kind: StartupDeadline, Detail: "start"}
	}
	return &ServiceExit{Service: service, Reason: reason}
}

func answerServiceRequests(
	ctx context.Context,
	service string,
	numbers NumberSource,
	serviceArtifacts *serviceArtifacts,
	numberStream, artifactStream *cmdsdk.RequestStream,
	streamWork *sync.WaitGroup,
) {
	streamWork.Add(2)
	go func() {
		defer streamWork.Done()
		err := numberStream.AnswerNumbers(
			ctx,
			func(ctx context.Context, q commandwire.NumbersRequest) (uint64, error) {
				return numbers.Reserve(ctx, q.Conversation, q.Sequence, q.Count)
			},
		)
		if err != nil && ctx.Err() == nil {
			slog.Warn("service " + service + "'s numbers stream broke: " + err.Error())
		}
	}()
	go func() {
		defer streamWork.Done()
		if err := artifactStream.AnswerArtifacts(
			ctx,
			serviceArtifacts.answer,
		); err != nil &&
			ctx.Err() == nil {
			slog.Warn("service " + service + "'s artifacts stream broke: " + err.Error())
		}
	}()
}

func waitService(
	life *serviceLife,
	service string,
	client *cmdsdk.Client,
	exited <-chan struct{},
	connection *watchedConnection,
	reaped *bool,
) error {
	select {
	case <-life.stop.Done():
		*reaped = shutdownService(context.Background(), client, exited, "service:"+service)
		return &RuntimeError{Kind: Stopped}
	case <-exited:
		*reaped = true
		return &ServiceExit{Service: service, Reason: ExitReason{Kind: ProcessExited}}
	case <-connection.ended:
		timer := time.NewTimer(250 * time.Millisecond)
		defer timer.Stop()
		select {
		case <-exited:
			*reaped = true
			return &ServiceExit{Service: service, Reason: ExitReason{Kind: ProcessExited}}
		case <-timer.C:
			return &ServiceExit{
				Service: service,
				Reason:  ExitReason{Kind: ProtocolBroken, Detail: connection.err.Error()},
			}
		}
	}
}

type serviceCleanup struct {
	stopStreams context.CancelFunc
	client      *cmdsdk.Client
	connection  *watchedConnection
	reaped      bool
	child       *process.Command
	service     string
	exited      <-chan struct{}
	streamWork  *sync.WaitGroup
	artifacts   *serviceArtifacts
	drained     <-chan struct{}
	diagnostic  *os.File
	result      error
	tail        *process.Tail
	state       *serviceWait
}

// finishService keeps the connection and stderr drain alive until shutdown has reaped the child.
func finishService(cleanup serviceCleanup) {
	cleanup.stopStreams()
	if cleanup.client != nil {
		_ = cleanup.client.Close()
	} else {
		_ = cleanup.connection.Close()
	} // Closing interrupts every outstanding stream.
	if !cleanup.reaped {
		if err := cleanup.child.Kill(); err != nil {
			slog.Warn("service:" + cleanup.service + " could not be killed: " + err.Error())
		}
		<-cleanup.exited
	}
	cleanup.streamWork.Wait()
	cleanup.artifacts.close()
	timer := time.NewTimer(250 * time.Millisecond)
	select {
	case <-cleanup.drained:
	case <-timer.C:
		_ = cleanup.diagnostic.Close()
		<-cleanup.drained
	}
	timer.Stop()
	var exit *ServiceExit
	if errors.As(cleanup.result, &exit) {
		exit.Stderr = cleanup.tail.Text()
		if exit.Reason.Kind == ProcessExited {
			exit.Reason.State = cleanup.state.exit
			exit.Reason.WaitErr = cleanup.state.err
		}
	}
}

// servicePipes appends each acquired endpoint immediately so the caller owns partial setup cleanup.
func servicePipes(cmd *exec.Cmd, pipes *[]*os.File) (*os.File, *os.File, *os.File, error) {
	stdin, input, err := os.Pipe()
	if err != nil {
		return nil, nil, nil, err
	}
	*pipes = append(*pipes, stdin, input)
	output, stdout, err := os.Pipe()
	if err != nil {
		return nil, nil, nil, err
	}
	*pipes = append(*pipes, output, stdout)
	diagnostic, stderr, err := os.Pipe()
	if err != nil {
		return nil, nil, nil, err
	}
	*pipes = append(*pipes, diagnostic, stderr)
	cmd.Stdin, cmd.Stdout, cmd.Stderr = stdin, stdout, stderr
	return input, output, diagnostic, nil
}

// startServiceChild limits startup cancellation to the attempt; a running service shuts down gracefully.
func startServiceChild(ctx, stop context.Context, child *process.Command, cancelChild context.CancelFunc) error {
	// Start may retry a busy executable. Once started, stop requests graceful
	// shutdown rather than cancelling the process owner's lifetime immediately.
	startInterrupted := make(chan struct{})
	stopStart := context.AfterFunc(stop, func() {
		cancelChild()
		close(startInterrupted)
	})
	err := child.Start(ctx)
	if !stopStart() {
		<-startInterrupted
	}
	if err != nil {
		return err
	}
	return nil
}

func (r *ServiceRegistry) serviceCommand(executable string) *exec.Cmd {
	cmd := exec.Command(executable, cmdsdk.CommandService)
	cmd.Dir = r.cwd
	cmd.Env = make([]string, 0, len(r.env))
	for key, value := range r.env {
		cmd.Env = append(cmd.Env, key+"="+value)
	}
	return cmd
}

func (r *ServiceRegistry) publishService(
	life *serviceLife,
	client *cmdsdk.Client,
	info commandwire.ServiceInfo,
	connection *watchedConnection,
) {
	r.mu.Lock()
	life.client, life.info = client, info
	life.connectionEnded = connection.ended
	close(life.ready)
	r.mu.Unlock()
}

func (r *ServiceRegistry) serveService(
	ctx context.Context,
	life *serviceLife,
	descriptor commandwire.PackageDescriptor,
	numbers NumberSource,
	child *process.Command,
	input, output, diagnostic *os.File,
) (result error) {
	state, exited, tail, drained := observeService(ctx, child, descriptor.ID, diagnostic)
	connection := &watchedConnection{Conn: &cmdsdk.PipeConn{Reader: output, Writer: input}, ended: make(chan struct{})}
	var client *cmdsdk.Client
	var streamWork sync.WaitGroup
	streamctx, stopStreams := context.WithCancel(context.Background())
	serviceArtifacts := &serviceArtifacts{registry: r, pkg: descriptor.ID}
	reaped := false
	defer func() {
		finishService(serviceCleanup{
			stopStreams: stopStreams,
			client:      client,
			connection:  connection,
			reaped:      reaped,
			child:       child,
			service:     descriptor.ID,
			exited:      exited,
			streamWork:  &streamWork,
			artifacts:   serviceArtifacts,
			drained:     drained,
			diagnostic:  diagnostic,
			result:      result,
			tail:        tail,
			state:       state,
		})
	}()
	startctx, cancelStart := context.WithCancelCause(streamctx)
	defer cancelStart(context.Canceled)
	client, info, numberStream, artifactStream, startErr, err := connectService(
		startctx,
		cancelStart,
		life,
		descriptor,
		connection,
	)
	if err != nil {
		return failedServiceStart(life, descriptor.ID, client, exited, &reaped, startErr, err)
	}
	answerServiceRequests(
		streamctx,
		descriptor.ID,
		numbers,
		serviceArtifacts,
		numberStream,
		artifactStream,
		&streamWork,
	)
	slog.Info(fmt.Sprintf("service %s started (pid %d)", descriptor.ID, child.PID()))
	r.publishService(life, client, info, connection)
	return waitService(life, descriptor.ID, client, exited, connection, &reaped)
}

// serviceWait is what reaping the service process reported.
type serviceWait struct {
	exit process.Exit
	err  error
}

// observeService starts the reaper and stderr drain; serveService joins both through finishService.
func observeService(
	ctx context.Context,
	child *process.Command,
	service string,
	diagnostic *os.File,
) (*serviceWait, <-chan struct{}, *process.Tail, <-chan struct{}) {
	exited := make(chan struct{})
	state := new(serviceWait)
	go func() {
		state.exit, state.err = child.Wait(ctx)
		close(exited)
	}()
	tail := process.NewTail(runnerwire.ServiceStderrChars)
	drained := make(chan struct{})
	go drainStderr(diagnostic, service, tail, drained)
	return state, exited, tail, drained
}
