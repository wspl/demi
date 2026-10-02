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

func (r *ServiceRegistry) live(life *serviceLife, descriptor commandwire.PackageDescriptor, artifact commandwire.PackageArtifact, resolver ArtifactResolver, numbers NumberSource) {
	defer r.work.Done()
	defer life.cancel()
	hold := r.cache.Holds().Hold(artifact.SHA256)
	defer hold.Release()
	executable, err := r.cache.Install(life.stop, Wanted{Package: descriptor.ID, Name: "program", Version: descriptor.Version, Artifact: artifact, Form: &commandwire.ArtifactFile{}}, resolver)
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
func (r *ServiceRegistry) runService(life *serviceLife, descriptor commandwire.PackageDescriptor, executable string, numbers NumberSource) (result error) {
	cmd := exec.Command(executable, cmdsdk.CommandService)
	cmd.Dir = r.cwd
	cmd.Env = make([]string, 0, len(r.env))
	for key, value := range r.env {
		cmd.Env = append(cmd.Env, key+"="+value)
	}
	// Caller-owned pipes let the stderr drain finish after Wait reaps the child.
	var pipes []*os.File
	defer func() {
		for _, pipe := range pipes {
			_ = pipe.Close()
		}
	}() // Already-closed ends are harmless; the result describes the service.
	stdin, input, err := os.Pipe()
	if err != nil {
		return err
	}
	pipes = append(pipes, stdin, input)
	output, stdout, err := os.Pipe()
	if err != nil {
		return err
	}
	pipes = append(pipes, output, stdout)
	diagnostic, stderr, err := os.Pipe()
	if err != nil {
		return err
	}
	pipes = append(pipes, diagnostic, stderr)
	cmd.Stdin, cmd.Stdout, cmd.Stderr = stdin, stdout, stderr
	child := process.Wrap(cmd, true, process.ChildAttributes{})
	childctx, cancelChild := context.WithCancel(context.Background())
	defer cancelChild()
	// Start may retry a busy executable. Once started, life.stop requests graceful
	// shutdown rather than cancelling the process owner's lifetime immediately.
	startInterrupted := make(chan struct{})
	stopStart := context.AfterFunc(life.stop, func() {
		cancelChild()
		close(startInterrupted)
	})
	err = child.Start(childctx)
	if !stopStart() {
		<-startInterrupted
	}
	if err != nil {
		return err
	}
	for _, pipe := range []*os.File{stdin, stdout, stderr} {
		_ = pipe.Close()
	} // Parent no longer owns the child ends.
	exited := make(chan struct{})
	var state process.Exit
	go func() {
		state = child.Wait(childctx)
		close(exited)
	}()
	tail := process.NewTail(runnerwire.ServiceStderrChars)
	drained := make(chan struct{})
	go drainStderr(diagnostic, descriptor.ID, tail, drained)
	connection := &watchedConnection{Conn: &cmdsdk.PipeConn{Reader: output, Writer: input}, ended: make(chan struct{})}
	var client *cmdsdk.Client
	var streamWork sync.WaitGroup
	streamctx, stopStreams := context.WithCancel(context.Background())
	serviceArtifacts := &serviceArtifacts{registry: r, pkg: descriptor.ID}
	reaped := false
	defer func() {
		stopStreams()
		if client != nil {
			_ = client.Close()
		} else {
			_ = connection.Close()
		} // Closing interrupts every outstanding stream.
		if !reaped {
			if err := child.Kill(); err != nil {
				slog.Warn("service:" + descriptor.ID + " could not be killed: " + err.Error())
			}
			<-exited
		}
		streamWork.Wait()
		serviceArtifacts.close()
		timer := time.NewTimer(250 * time.Millisecond)
		select {
		case <-drained:
		case <-timer.C:
			_ = diagnostic.Close()
			<-drained
		}
		timer.Stop()
		var exit *ServiceExit
		if errors.As(result, &exit) {
			exit.Stderr = tail.Text()
			if exit.Reason.Kind == ProcessExited {
				exit.Reason.State = state
			}
		}
	}()
	startctx, cancelStart := context.WithCancelCause(streamctx)
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
	defer cancelStart(context.Canceled)
	client, err = cmdsdk.Connect(startctx, connection)
	var info commandwire.ServiceInfo
	var numberStream, artifactStream *cmdsdk.RequestStream
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
	startErr := context.Cause(startctx)
	if err == nil {
		err = startErr
	}
	if err != nil {
		// A connected startup has the same shutdown obligations as a ready
		// service: it may already hold resources despite a bad catalog or
		// an unanswered startup request. Keep its connection and stderr alive
		// through the shutdown deadline; the finalizer kills only afterward.
		exitedBeforeStop := false
		select {
		case <-exited:
			reaped = true
			exitedBeforeStop = true
		default:
		}
		if client != nil && !reaped {
			reaped = shutdownService(context.Background(), client, exited, "service:"+descriptor.ID)
		}
		if life.stop.Err() != nil {
			return &RuntimeError{Kind: Cancelled, Cause: life.stop.Err()}
		}
		if exitedBeforeStop {
			return &ServiceExit{Service: descriptor.ID, Reason: ExitReason{Kind: ProcessExited}}
		}
		var runtimeErr *RuntimeError
		if errors.As(err, &runtimeErr) {
			return err
		}
		reason := ExitReason{Kind: ProtocolBroken, Detail: err.Error()}
		if errors.Is(startErr, context.DeadlineExceeded) {
			reason = ExitReason{Kind: StartupDeadline, Detail: "start"}
		}
		return &ServiceExit{Service: descriptor.ID, Reason: reason}
	}
	streamWork.Add(2)
	go func() {
		defer streamWork.Done()
		err := numberStream.AnswerNumbers(streamctx, func(ctx context.Context, q commandwire.NumbersRequest) (uint64, error) {
			return numbers.Reserve(ctx, q.Conversation, q.Sequence, q.Count)
		})
		if err != nil && streamctx.Err() == nil {
			slog.Warn("service " + descriptor.ID + "'s numbers stream broke: " + err.Error())
		}
	}()
	go func() {
		defer streamWork.Done()
		if err := artifactStream.AnswerArtifacts(streamctx, serviceArtifacts.answer); err != nil && streamctx.Err() == nil {
			slog.Warn("service " + descriptor.ID + "'s artifacts stream broke: " + err.Error())
		}
	}()
	slog.Info(fmt.Sprintf("service %s started (pid %d)", descriptor.ID, child.PID()))
	r.mu.Lock()
	life.client, life.info = client, info
	life.connectionEnded = connection.ended
	close(life.ready)
	r.mu.Unlock()
	select {
	case <-life.stop.Done():
		reaped = shutdownService(context.Background(), client, exited, "service:"+descriptor.ID)
		return &RuntimeError{Kind: Stopped}
	case <-exited:
		reaped = true
		return &ServiceExit{Service: descriptor.ID, Reason: ExitReason{Kind: ProcessExited}}
	case <-connection.ended:
		timer := time.NewTimer(250 * time.Millisecond)
		defer timer.Stop()
		select {
		case <-exited:
			reaped = true
			return &ServiceExit{Service: descriptor.ID, Reason: ExitReason{Kind: ProcessExited}}
		case <-timer.C:
			return &ServiceExit{Service: descriptor.ID, Reason: ExitReason{Kind: ProtocolBroken, Detail: connection.err.Error()}}
		}
	}
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
	if line := lines.Finish(); line != nil {
		slog.Info(*line, "source", "service:"+service)
	}
}

func connectionLost(err error) bool {
	var op *net.OpError
	var path *os.PathError
	return errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) || errors.Is(err, net.ErrClosed) || errors.Is(err, os.ErrClosed) || errors.As(err, &op) || errors.As(err, &path)
}

// shutdownService asks a connected service to release its resources, giving the
// request and process exit one shared six-second deadline, as the Rust owner does.
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
