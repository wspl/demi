package hostremote

import (
	"context"
	"fmt"
	"io"
	"sync"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/gates"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

// Transport is one authenticated runner connection. Close interrupts Read and
// Write. Its owner has already read and admitted the runner's hello.
type Transport interface {
	Read(context.Context) ([]byte, error)
	Write(context.Context, []byte) error
	Close() error
}
type LinkOptions struct {
	Device   string
	Identity shell.HostIdentity
	Pipes    *Pipes
	Policy   LinkPolicy
	Ping     time.Duration // zero disables liveness
}
type LinkEnd struct{ Kind, Reason string }
type Link struct {
	settled            bool
	mu                 sync.Mutex
	device             string
	identity           shell.HostIdentity
	pipes              *Pipes
	policy             LinkPolicy
	ctx                context.Context
	cancel             context.CancelFunc
	reason             string
	outbound           chan []byte
	waiting            map[string]*replyWaiter
	jobs               map[string]*jobEntry
	spawns             map[string]*spawnEntry
	services           map[string]*serviceEntry
	calls              map[string]*callEntry
	artifacts, numbers map[string]bool
	manifest           string
	pongJobs           uint64
	liveness           string
	tasks              sync.WaitGroup
	jobTurn            *gates.SerialGate
}
type LinkDriver struct {
	link *Link
	ping time.Duration
	Tap  chan<- runnerproto.Outbound
}
type reply struct {
	value any
	err   error
}
type replyWaiter struct {
	expected string
	reply    chan reply
}
type jobEntry struct {
	shared   *shared[JobEnd, JobOutput]
	origin   JobOrigin
	commands *CommandSelection
	ctx      context.Context
	cancel   context.CancelFunc
	release  func()
}
type spawnEntry struct {
	shared   *shared[shell.ProcessEnd, shell.ProcessOutput]
	retained bool
	release  func()
}
type serviceEntry struct {
	descriptor commandservice.PackageDescriptor
	resolver   ArtifactResolver
	ctx        context.Context
	cancel     context.CancelFunc
	done       chan serviceAnswer
}
type serviceAnswer struct {
	end ServiceEnd
	err error
}

func NewLink(options LinkOptions) (*Link, *LinkDriver) {
	ctx, cancel := context.WithCancel(context.Background())
	l := &Link{device: options.Device, identity: options.Identity, pipes: options.Pipes, policy: options.Policy, ctx: ctx, cancel: cancel, outbound: make(chan []byte, OutboundFrames), waiting: map[string]*replyWaiter{}, jobs: map[string]*jobEntry{}, spawns: map[string]*spawnEntry{}, services: map[string]*serviceEntry{}, calls: map[string]*callEntry{}, artifacts: map[string]bool{}, numbers: map[string]bool{}, jobTurn: gates.NewSerialGate()}
	return l, &LinkDriver{link: l, ping: options.Ping}
}
func (l *Link) Device() string               { return l.device }
func (l *Link) Identity() shell.HostIdentity { return l.identity }
func (l *Link) Pipes() *Pipes                { return l.pipes }
func (l *Link) IsClosed() bool               { return l.ctx.Err() != nil }
func (l *Link) offline() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	reason := l.reason
	if reason == "" {
		reason = "runner disconnected"
	}
	return &shell.HostError{Kind: shell.HostOffline, Message: reason}
}
func (l *Link) PauseLiveness() {
	l.mu.Lock()
	l.liveness = "paused"
	l.mu.Unlock()
}
func (l *Link) ResumeLiveness() {
	l.mu.Lock()
	l.liveness = ""
	l.mu.Unlock()
}
func (l *Link) RunningJobs() uint64 {
	l.mu.Lock()
	defer l.mu.Unlock()
	count := uint64(len(l.jobs))
	for _, s := range l.spawns {
		if !s.retained {
			count++
		}
	}
	return max(count, l.pongJobs)
}
func (l *Link) Disconnect(reason string) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if !l.IsClosed() {
		l.reason = reason
		l.cancel()
	}
}
func (l *Link) Sync(ctx context.Context) error {
	_, err := l.call(ctx, "Sync", func(id string) runnerproto.Inbound { return runnerproto.InboundSync{ID: id} })
	return err
}
func (l *Link) ReleaseConversation(ctx context.Context, conversation string) error {
	ctx, cancel := context.WithTimeout(ctx, 360*time.Second)
	defer cancel()
	_, err := l.call(ctx, "Release", func(id string) runnerproto.Inbound {
		return runnerproto.InboundConversationRelease{ID: id, ConversationID: conversation}
	})
	if ctx.Err() == context.DeadlineExceeded {
		return &shell.HostError{Kind: shell.HostInterrupted, Message: "Conversation release timed out"}
	}
	return err
}
func (l *Link) spawn(fn func()) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.IsClosed() {
		return false
	}
	l.tasks.Add(1)
	go func() {
		defer l.tasks.Done()
		fn()
	}()
	return true
}
func (l *Link) send(ctx context.Context, message runnerproto.Inbound) error {
	bytes, err := runnerproto.EncodeInboundMsgpack(message)
	if err != nil {
		return protocolError(err.Error())
	}
	if len(bytes) > runnerproto.MaxMessageBytes {
		return &shell.HostError{Kind: shell.HostTooLarge, Message: fmt.Sprintf("the request is %d bytes, over the %d-byte message limit", len(bytes), runnerproto.MaxMessageBytes)}
	}
	if l.IsClosed() {
		return l.offline()
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-l.ctx.Done():
		return l.offline()
	case l.outbound <- bytes:
		return nil
	}
}
func (l *Link) post(message runnerproto.Inbound) {
	l.spawn(func() {
		// Teardown releases all remote work; a late control message has no owner.
		_ = l.send(l.ctx, message)
	})
}
func (l *Link) call(ctx context.Context, expected string, message func(string) runnerproto.Inbound) (any, error) {
	return l.callID(ctx, requestID(), expected, message)
}
func (l *Link) callID(ctx context.Context, id, expected string, message func(string) runnerproto.Inbound) (any, error) {
	waiter := &replyWaiter{expected: expected, reply: make(chan reply, 1)}
	l.mu.Lock()
	if l.IsClosed() {
		l.mu.Unlock()
		return nil, l.offline()
	}
	l.waiting[id] = waiter
	l.mu.Unlock()
	defer func() {
		l.mu.Lock()
		delete(l.waiting, id)
		l.mu.Unlock()
	}()
	if err := l.send(ctx, message(id)); err != nil {
		return nil, err
	}
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case answer := <-waiter.reply:
		return answer.value, answer.err
	}
}
func (l *Link) answer(id, expected string, value any, err error) {
	l.mu.Lock()
	waiter := l.waiting[id]
	delete(l.waiting, id)
	l.mu.Unlock()
	if waiter == nil {
		return
	}
	if err == nil && waiter.expected != expected {
		err = protocolError(fmt.Sprintf("the runner answered %s to a %s request", expected, waiter.expected))
	}
	waiter.reply <- reply{value: value, err: err}
}
func protocolError(message string) error {
	return &shell.HostError{Kind: shell.HostProtocol, Message: message}
}
func (d *LinkDriver) Serve(ctx context.Context, transport Transport) LinkEnd {
	l := d.link
	ends := make(chan LinkEnd, 2)
	var ioTasks sync.WaitGroup
	ioTasks.Add(2)
	go func() {
		defer ioTasks.Done()
		for {
			frame, err := transport.Read(l.ctx)
			if err != nil {
				ends <- LinkEnd{"closed", "runner disconnected: " + err.Error()}
				return
			}
			message, err := runnerproto.DecodeOutboundMsgpack(frame)
			if err != nil {
				ends <- LinkEnd{"refused", err.Error()}
				return
			}
			if d.Tap != nil {
				select {
				case d.Tap <- message:
				default:
				}
			}
			l.receive(message)
		}
	}()
	go func() {
		defer ioTasks.Done()
		for {
			select {
			case <-l.ctx.Done():
				return
			case frame := <-l.outbound:
				if err := transport.Write(l.ctx, frame); err != nil {
					ends <- LinkEnd{"closed", "runner disconnected: " + err.Error()}
					return
				}
			}
		}
	}()
	var tick <-chan time.Time
	if d.ping > 0 {
		ticker := time.NewTicker(d.ping)
		defer ticker.Stop()
		tick = ticker.C
	}
	var end LinkEnd
loop:
	for {
		select {
		case end = <-ends:
			break loop
		case <-ctx.Done():
			end = LinkEnd{"disconnected", ctx.Err().Error()}
			break loop
		case <-l.ctx.Done():
			end = LinkEnd{"disconnected", l.offline().Error()}
			break loop
		case <-tick:
			l.mu.Lock()
			phase := l.liveness
			if phase == "" {
				l.liveness = "waiting"
			}
			l.mu.Unlock()
			switch phase {
			case "waiting":
				end = LinkEnd{"disconnected", "liveness: ping unanswered"}
				break loop
			case "":
				l.post(runnerproto.InboundPing{})
			}
		}
	}
	reason := end.Reason
	if end.Kind != "disconnected" {
		reason = "runner disconnected"
	}
	l.teardown(reason)
	// The transport is owned by this driver; closing it interrupts both IO loops.
	_ = transport.Close()
	ioTasks.Wait()
	l.tasks.Wait()
	return end
}
func (l *Link) teardown(reason string) {
	l.mu.Lock()
	if l.settled {
		l.mu.Unlock()
		return
	}
	l.settled = true
	l.reason = reason
	l.cancel()
	waiting, jobs, spawns, services, calls := l.waiting, l.jobs, l.spawns, l.services, l.calls
	l.waiting = map[string]*replyWaiter{}
	l.jobs = map[string]*jobEntry{}
	l.spawns = map[string]*spawnEntry{}
	l.services = map[string]*serviceEntry{}
	l.calls = map[string]*callEntry{}
	l.mu.Unlock()
	for _, w := range waiting {
		w.reply <- reply{err: l.offline()}
	}
	for _, job := range jobs {
		job.cancel()
		job.shared.finish(lostJob(reason))
		if job.release != nil {
			job.release()
		}
	}
	for _, spawn := range spawns {
		spawn.shared.finish(shell.ProcessEnd{Kind: shell.ProcessLost, Reason: reason})
		if spawn.release != nil {
			spawn.release()
		}
	}
	for _, s := range services {
		s.cancel()
		select {
		case s.done <- serviceAnswer{err: l.offline()}:
		default:
		}
	}
	for _, call := range calls {
		call.stop(reason, false)
	}
	l.pipes.DeviceGone(l.device)
}

// shared routes output without waiting for its consumer, as the Rust connection
// does. The owner finishes it exactly once; readers drain queued output first.
type shared[E, C any] struct {
	mu      sync.Mutex
	output  []C
	end     *E
	hints   []runningHint
	changed chan struct{}
}
type runningHint struct{ id, value string }

func newShared[E, C any]() *shared[E, C] { return &shared[E, C]{changed: make(chan struct{})} }
func (s *shared[E, C]) signal() {
	close(s.changed)
	s.changed = make(chan struct{})
}
func (s *shared[E, C]) push(chunk C) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.end == nil {
		s.output = append(s.output, chunk)
		s.signal()
	}
}
func (s *shared[E, C]) finish(end E) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.end == nil {
		s.end = &end
		s.hints = nil
		s.signal()
	}
}
func (s *shared[E, C]) ended() *E {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.end == nil {
		return nil
	}
	value := *s.end
	return &value
}
func (s *shared[E, C]) next(ctx context.Context) (C, error) {
	var zero C
	for {
		s.mu.Lock()
		if len(s.output) > 0 {
			chunk := s.output[0]
			s.output[0] = zero
			s.output = s.output[1:]
			s.mu.Unlock()
			return chunk, nil
		}
		ended, changed := s.end != nil, s.changed
		s.mu.Unlock()
		if ended {
			return zero, io.EOF
		}
		select {
		case <-ctx.Done():
			return zero, ctx.Err()
		case <-changed:
		}
	}
}
func (s *shared[E, C]) wait(ctx context.Context) (E, error) {
	var zero E
	for {
		s.mu.Lock()
		if s.end != nil {
			end := *s.end
			s.mu.Unlock()
			return end, nil
		}
		changed := s.changed
		s.mu.Unlock()
		select {
		case <-ctx.Done():
			return zero, ctx.Err()
		case <-changed:
		}
	}
}
func (s *shared[E, C]) hint(id string, value *string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.end != nil {
		return
	}
	for i, h := range s.hints {
		if h.id == id {
			if value == nil {
				s.hints = append(s.hints[:i], s.hints[i+1:]...)
			} else {
				s.hints[i].value = *value
			}
			return
		}
	}
	if value != nil {
		s.hints = append(s.hints, runningHint{id, *value})
	}
}
func (s *shared[E, C]) runningHint() *string {
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.hints) == 0 {
		return nil
	}
	return new(s.hints[len(s.hints)-1].value)
}

func (l *Link) receive(message runnerproto.Outbound) {
	switch m := message.(type) {
	case runnerproto.OutboundHello:
	case runnerproto.OutboundPong:
		l.mu.Lock()
		l.pongJobs = m.Jobs
		if l.liveness == "waiting" {
			l.liveness = ""
		}
		l.mu.Unlock()
	case runnerproto.OutboundFSOk:
		l.answer(m.ID, fsExpected(m.Result), m.Result, nil)
	case runnerproto.OutboundFSError:
		l.answer(m.ID, "", nil, fsError(m.Code, m.Message))
	case runnerproto.OutboundGitOk:
		l.answer(m.ID, gitExpected(m.Result), m.Result, nil)
	case runnerproto.OutboundGitError:
		l.answer(m.ID, "", nil, fsError(&m.Code, m.Message))
	case runnerproto.OutboundConversationReleased:
		l.doneAnswer(m.ID, "Release", m.Error)
	case runnerproto.OutboundJobRead:
		l.doneAnswer(m.ID, "JobRead", m.Error)
	case runnerproto.OutboundSyncDone:
		l.doneAnswer(m.ID, "Sync", m.Error)
	case runnerproto.OutboundLogLines:
		l.answer(m.ID, "Log", LogPage{m.Lines, m.Next}, nil)
	case runnerproto.OutboundLogError:
		l.answer(m.ID, "", nil, fsError(nil, m.Message))
	case runnerproto.OutboundNetOpened:
		l.answer(m.StreamID, "Net", nil, nil)
	case runnerproto.OutboundServiceOpened:
		l.answer(m.StreamID, "Service", nil, nil)
	case runnerproto.OutboundNetError:
		code := string(m.Code)
		l.answer(m.StreamID, "", nil, fsError(&code, m.Message))
	case runnerproto.OutboundServiceError:
		code := string(m.Code)
		l.answer(m.StreamID, "", nil, fsError(&code, m.Message))
	case runnerproto.OutboundServiceDone:
		l.mu.Lock()
		s := l.services[m.StreamID]
		l.mu.Unlock()
		if s != nil {
			select {
			case s.done <- serviceAnswer{end: ServiceEnd{m.ExitCode, m.Stderr}}:
			default:
			}
		}
	case runnerproto.OutboundSpawnOutput:
		l.mu.Lock()
		s := l.spawns[m.SpawnID]
		l.mu.Unlock()
		if s != nil {
			s.shared.push(shell.ProcessOutput{Stream: core.StreamKind(m.Stream), Bytes: m.Bytes})
		}
	case runnerproto.OutboundSpawnExit:
		l.mu.Lock()
		s := l.spawns[m.SpawnID]
		delete(l.spawns, m.SpawnID)
		l.mu.Unlock()
		if s != nil {
			s.shared.finish(processEnd(m.ExitCode, m.Signal, m.SpawnError))
			if s.release != nil {
				l.releaseAfterReply(s.release)
			}
		}
	case runnerproto.OutboundJobOutput:
		l.mu.Lock()
		j := l.jobs[m.JobID]
		l.mu.Unlock()
		if j != nil {
			j.shared.push(JobOutput{core.StreamKind(m.Stream), m.Offset, m.Bytes})
		}
	case runnerproto.OutboundJobRunningHint:
		l.mu.Lock()
		j := l.jobs[m.JobID]
		l.mu.Unlock()
		if j != nil {
			j.shared.hint(m.InvocationID, m.Hint)
		}
	case runnerproto.OutboundJobExit:
		l.mu.Lock()
		j := l.jobs[m.JobID]
		delete(l.jobs, m.JobID)
		var calls []*callEntry
		for _, c := range l.calls {
			if c.jobID == m.JobID {
				calls = append(calls, c)
			}
		}
		l.mu.Unlock()
		for _, c := range calls {
			c.stop("calling job "+m.JobID+" exited before its RPC completed", false)
		}
		if j != nil {
			j.cancel()
			j.shared.finish(JobEnd{processEnd(m.ExitCode, m.Signal, m.SpawnError), m.Cwd, m.Output, m.Files, m.FilesTruncated})
			if j.release != nil {
				l.releaseAfterReply(j.release)
			}
		}
	case runnerproto.OutboundPipeDone:
		if !m.Ok {
			reason := "device transfer failed"
			if m.Error != nil {
				reason = *m.Error
			}
			l.pipes.FailFromDevice(m.PipeID, l.device, reason)
		}
	case runnerproto.OutboundRPCCall:
		l.startRPC(m)
	case runnerproto.OutboundRPCStdin:
		l.mu.Lock()
		c := l.calls[m.CallID]
		l.mu.Unlock()
		if c != nil {
			c.liveInput(m.Bytes)
		}
	case runnerproto.OutboundRPCStdinEnd:
		l.mu.Lock()
		c := l.calls[m.CallID]
		l.mu.Unlock()
		if c != nil {
			c.endLiveInput()
		}
	case runnerproto.OutboundRPCCancel:
		l.mu.Lock()
		c := l.calls[m.CallID]
		l.mu.Unlock()
		if c != nil {
			c.stop("command cancelled", true)
		}
	case runnerproto.OutboundArtifactResolve:
		l.resolveArtifact(m)
	case runnerproto.OutboundNumbersReserve:
		l.reserveNumbers(m)
	case runnerproto.OutboundVolumeGrow:
		l.spawn(func() {
			err := l.policy.GrowVolume(l.ctx, m.Volume, m.Bytes)
			var why *string
			if err != nil {
				why = new(err.Error())
			}
			_ = l.send(l.ctx, runnerproto.InboundVolumeGrown{ID: m.ID, Volume: m.Volume, Bytes: m.Bytes, Error: why})
		})
	}
}
func (l *Link) doneAnswer(id, expected string, reason *string) {
	var err error
	if reason != nil {
		err = fsError(nil, *reason)
	}
	l.answer(id, expected, nil, err)
}

// fsExpected names the typed reply as the Rust diagnostic prints it.
func fsExpected(result runnerproto.FSResult) string {
	switch result.(type) {
	case runnerproto.FSResultReadFile:
		return `Fs("readFile")`
	case runnerproto.FSResultWriteFile:
		return `Fs("writeFile")`
	case runnerproto.FSResultExists:
		return `Fs("exists")`
	case runnerproto.FSResultStat:
		return `Fs("stat")`
	case runnerproto.FSResultLstat:
		return `Fs("lstat")`
	case runnerproto.FSResultReaddir:
		return `Fs("readdir")`
	case runnerproto.FSResultMkdir:
		return `Fs("mkdir")`
	case runnerproto.FSResultRm:
		return `Fs("rm")`
	case runnerproto.FSResultCp:
		return `Fs("cp")`
	case runnerproto.FSResultMv:
		return `Fs("mv")`
	case runnerproto.FSResultChmod:
		return `Fs("chmod")`
	case runnerproto.FSResultSymlink:
		return `Fs("symlink")`
	case runnerproto.FSResultLink:
		return `Fs("link")`
	case runnerproto.FSResultReadlink:
		return `Fs("readlink")`
	case runnerproto.FSResultRealpath:
		return `Fs("realpath")`
	case runnerproto.FSResultUtimes:
		return `Fs("utimes")`
	}
	return "Fs"
}
func gitExpected(result runnerproto.GitResult) string {
	switch result.(type) {
	case runnerproto.GitResultChanges:
		return `Git("changes")`
	case runnerproto.GitResultShow:
		return `Git("show")`
	}
	return "Git"
}

// releaseAfterReply keeps routing independent of the shard accepting a lease release.
// Only the reader calls this; Serve joins it before waiting for these tasks.
func (l *Link) releaseAfterReply(release func()) {
	l.tasks.Add(1)
	go func() {
		defer l.tasks.Done()
		release()
	}()
}
