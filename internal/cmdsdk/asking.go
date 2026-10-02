package cmdsdk

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"sync"

	"github.com/wspl/demi/internal/commandwire"
)

type answer[T any] struct {
	value T
	err   error
}

// Pending is a service request awaiting its runner's answer. Reply may be called once.
type Pending[Q, R any] struct {
	Request Q
	reply   chan answer[R]
}

// Reply delivers a result even if the original caller has stopped waiting.
func (p Pending[Q, R]) Reply(value R, err error) {
	select {
	case p.reply <- answer[R]{value, err}:
	default:
	}
}

type asker[Q, R any] struct {
	requests chan Pending[Q, R]
	done     chan struct{}
	once     sync.Once
}

func newAsker[Q, R any]() *asker[Q, R] {
	return &asker[Q, R]{requests: make(chan Pending[Q, R], 64), done: make(chan struct{})}
}
func (a *asker[Q, R]) close() { a.once.Do(func() { close(a.done) }) }
func (a *asker[Q, R]) ask(ctx context.Context, q Q) (R, error) {
	var zero R
	p := Pending[Q, R]{Request: q, reply: make(chan answer[R], 1)}
	select {
	case <-ctx.Done():
		return zero, ctx.Err()
	case <-a.done:
		return zero, io.EOF
	case a.requests <- p:
	}
	select {
	case <-ctx.Done():
		return zero, ctx.Err()
	case <-a.done:
		return zero, io.EOF
	case r := <-p.reply:
		return r.value, r.err
	}
}

// Draw is one conversation number request waiting for an answer.
type Draw = Pending[commandwire.NumbersRequest, uint64]

// Numbers draws from conversation sequences through the runner.
type Numbers struct {
	source *asker[commandwire.NumbersRequest, uint64]
}

// NumbersChannel creates a bounded source and its answering endpoint.
func NumbersChannel() (*Numbers, <-chan Draw) {
	a := newAsker[commandwire.NumbersRequest, uint64]()
	return &Numbers{source: a}, a.requests
}

// Close ends the source and wakes all callers.
func (n *Numbers) Close() { n.source.close() }

// Draw reserves count consecutive numbers and returns the first.
func (n *Numbers) Draw(ctx context.Context, conversation string, sequence commandwire.ServiceSequence, count uint32) (uint64, error) {
	q := commandwire.NumbersRequest{Conversation: conversation, Sequence: sequence, Count: count}
	if err := q.Validate(); err != nil {
		return 0, err
	}
	value, err := n.source.ask(ctx, q)
	if err != nil {
		return 0, fmt.Errorf("conversation numbers: %w", err)
	}
	return value, nil
}

// ArtifactPending is an artifact request awaiting its runner's answer.
type ArtifactPending = Pending[commandwire.ArtifactRequest, commandwire.ArtifactAnswer]

// Artifacts asks the runner to install or list artifacts, never deriving their paths.
type Artifacts struct {
	source *asker[commandwire.ArtifactRequest, commandwire.ArtifactAnswer]
}

// ArtifactsChannel creates a bounded artifact source and its answering endpoint.
func ArtifactsChannel() (*Artifacts, <-chan ArtifactPending) {
	a := newAsker[commandwire.ArtifactRequest, commandwire.ArtifactAnswer]()
	return &Artifacts{source: a}, a.requests
}

// Close ends the source and wakes callers.
func (a *Artifacts) Close() { a.source.close() }
func (a *Artifacts) ask(ctx context.Context, q commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error) {
	if err := q.Validate(); err != nil {
		return commandwire.ArtifactAnswer{}, err
	}
	r, err := a.source.ask(ctx, q)
	if err != nil {
		return r, fmt.Errorf("artifact: %w", err)
	}
	if err := r.Validate(); err != nil {
		return r, err
	}
	if r.Error != nil {
		return r, errors.New(*r.Error)
	}
	return r, nil
}

// Install returns the path of an installed file or archive entry.
func (a *Artifacts) Install(ctx context.Context, install commandwire.ArtifactInstall) (string, error) {
	r, err := a.ask(ctx, commandwire.ArtifactRequest{Install: &install})
	if err != nil {
		return "", err
	}
	if r.Path == nil {
		return "", errors.New("runner answered another kind of artifact request")
	}
	return *r.Path, nil
}

// Installed returns the Host's artifacts of a line, newest first.
func (a *Artifacts) Installed(ctx context.Context, name string) ([]commandwire.InstalledArtifact, error) {
	r, err := a.ask(ctx, commandwire.ArtifactRequest{Installed: &commandwire.ArtifactsInstalled{Name: name}})
	if err != nil {
		return nil, err
	}
	if r.Installed == nil {
		return nil, errors.New("runner answered another kind of artifact request")
	}
	return *r.Installed, nil
}

type askCodec[Q, R, A any] struct {
	name             string
	rejectDuplicates bool
	request          func([]byte) (Q, error)
	identify         func(Q, uint64) Q
	id               func(Q) uint64
	answer           func(uint64, R, error) A
	decode           func([]byte) (A, error)
	outcome          func(A) (uint64, R, error)
}

var numberCodec = askCodec[commandwire.NumbersRequest, uint64, commandwire.NumbersAnswer]{
	rejectDuplicates: true,
	name:             "numbers",
	request:          commandwire.DecodeNumbersRequest,
	identify: func(q commandwire.NumbersRequest, id uint64) commandwire.NumbersRequest {
		q.ID = id
		return q
	},
	id: func(q commandwire.NumbersRequest) uint64 { return q.ID },
	answer: func(id, value uint64, err error) commandwire.NumbersAnswer {
		r := commandwire.NumbersAnswer{ID: id}
		if err != nil {
			s := err.Error()
			r.Error = &s
		} else {
			r.First = &value
		}
		return r
	},
	decode: commandwire.DecodeNumbersAnswer,
	outcome: func(a commandwire.NumbersAnswer) (uint64, uint64, error) {
		if a.Error != nil {
			return a.ID, 0, errors.New(*a.Error)
		}
		return a.ID, *a.First, nil
	},
}
var artifactCodec = askCodec[commandwire.ArtifactRequest, commandwire.ArtifactAnswer, commandwire.ArtifactAnswer]{
	name: "artifacts", request: commandwire.DecodeArtifactRequest,
	identify: func(q commandwire.ArtifactRequest, id uint64) commandwire.ArtifactRequest {
		q.ID = id
		return q
	},
	id: func(q commandwire.ArtifactRequest) uint64 { return q.ID },
	answer: func(id uint64, value commandwire.ArtifactAnswer, err error) commandwire.ArtifactAnswer {
		value.ID = id
		if err != nil {
			s := err.Error()
			return commandwire.ArtifactAnswer{ID: id, Error: &s}
		}
		return value
	},
	decode: commandwire.DecodeArtifactAnswer,
	outcome: func(a commandwire.ArtifactAnswer) (uint64, commandwire.ArtifactAnswer, error) {
		if a.Error != nil {
			return a.ID, a, errors.New(*a.Error)
		}
		return a.ID, a, nil
	},
}

func relayNumbers(ctx, finish context.Context, i *Input, o *Output, n *Numbers, q <-chan Draw) (commandwire.Completion, error) {
	return relay(ctx, finish, i, o, n.source, q, numberCodec)
}
func relayArtifacts(ctx, finish context.Context, i *Input, o *Output, a *Artifacts, q <-chan ArtifactPending) (commandwire.Completion, error) {
	return relay(ctx, finish, i, o, a.source, q, artifactCodec)
}

// relay correlates requests on one service side stream with their answers.
func relay[Q, R, A any](ctx, finish context.Context, i *Input, o *Output, source *asker[Q, R], requests <-chan Pending[Q, R], codec askCodec[Q, R, A]) (commandwire.Completion, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	defer source.close()
	stop := context.AfterFunc(finish, cancel)
	defer stop()
	var mu sync.Mutex
	waiting := map[uint64]Pending[Q, R]{}
	done := make(chan error, 2)
	go func() {
		var next uint64
		for {
			select {
			case <-ctx.Done():
				done <- ctx.Err()
				return
			case <-source.done:
				done <- nil
				return
			case p := <-requests:
				id := next
				next++
				mu.Lock()
				waiting[id] = p
				mu.Unlock()
				b, err := json.Marshal(codec.identify(p.Request, id))
				if err == nil {
					err = o.Stdout(ctx, b)
				}
				if err != nil {
					done <- err
					return
				}
			}
		}
	}()
	go func() {
		for {
			b, err := i.Next(ctx)
			if err != nil {
				done <- err
				return
			}
			a, err := codec.decode(b)
			if err != nil {
				done <- err
				return
			}
			id, value, outcome := codec.outcome(a)
			mu.Lock()
			p, ok := waiting[id]
			delete(waiting, id)
			mu.Unlock()
			if ok {
				p.Reply(value, outcome)
			}
		}
	}()
	err := <-done
	cancel()
	<-done
	if finish.Err() != nil || errors.Is(err, io.EOF) {
		err = nil
	}
	return commandwire.Completion{}, err
}

// RequestStream is one runner endpoint answering the service's requests.
type RequestStream struct {
	input  *CommandInput
	output *CommandOutput
}

func (c *Client) requestStream(ctx context.Context, path string) (*RequestStream, error) {
	b, err := (commandwire.StreamOpen{}).Encode()
	if err != nil {
		return nil, err
	}
	i, o, err := c.invokeAt(ctx, path, b, false)
	if err != nil {
		return nil, err
	}
	return &RequestStream{input: i, output: o}, nil
}

// Numbers opens the service's single numbers stream.
func (c *Client) Numbers(ctx context.Context) (*RequestStream, error) {
	return c.requestStream(ctx, commandwire.NumbersPath)
}

// Artifacts opens the service's single artifacts stream.
func (c *Client) Artifacts(ctx context.Context) (*RequestStream, error) {
	return c.requestStream(ctx, commandwire.ArtifactsPath)
}

// AnswerNumbers answers at most 32 requests concurrently, refusing excess requests.
func (s *RequestStream) AnswerNumbers(ctx context.Context, f func(context.Context, commandwire.NumbersRequest) (uint64, error)) error {
	return answerStream(ctx, s, numberCodec, f)
}

// AnswerArtifacts answers the artifact stream with runner-owned installations.
func (s *RequestStream) AnswerArtifacts(ctx context.Context, f func(context.Context, commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error)) error {
	return answerStream(ctx, s, artifactCodec, f)
}
func answerStream[Q, R, A any](ctx context.Context, s *RequestStream, codec askCodec[Q, R, A], f func(context.Context, Q) (R, error)) error {
	ctx, cancel := context.WithCancel(ctx)
	var workers sync.WaitGroup
	defer func() {
		cancel()
		s.input.Cancel()
		workers.Wait()
	}()
	records := make(chan commandwire.Record)
	failures := make(chan error, 1)
	workers.Add(1)
	go func() {
		defer workers.Done()
		for {
			r, err := s.output.Next(ctx)
			if err != nil {
				failures <- err
				return
			}
			select {
			case records <- r:
			case <-ctx.Done():
				return
			}
		}
	}()
	type response struct {
		id         uint64
		value      A
		releasesID bool
	}
	answers := make(chan response, 32)
	// IDs remain in flight until their original answer is sent, even after
	// its callback finishes. A duplicate refusal does not release that ID.
	inFlight := make(map[uint64]struct{})
	active := 0
	pulls := 0
	var queued []response
	for {
		for pulls > 0 && len(queued) > 0 {
			b, err := json.Marshal(queued[0].value)
			if err != nil {
				return err
			}
			if err = s.input.Write(ctx, b); err != nil {
				return err
			}
			if queued[0].releasesID {
				delete(inFlight, queued[0].id)
			}
			queued = queued[1:]
			pulls--
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case err := <-failures:
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		case a := <-answers:
			active--
			queued = append(queued, a)
		case r := <-records:
			switch r := r.(type) {
			case commandwire.Stdout:
				q, err := codec.request(r)
				if err != nil {
					return err
				}
				id := codec.id(q)
				if _, exists := inFlight[id]; codec.rejectDuplicates && exists {
					var zero R
					queued = append(queued, response{value: codec.answer(id, zero, fmt.Errorf("duplicate %s request id %d", codec.name, id))})
					continue
				}
				if active == 32 {
					var zero R
					queued = append(queued, response{value: codec.answer(id, zero, fmt.Errorf("too many %s requests in flight", codec.name))})
					continue
				}
				if codec.rejectDuplicates {
					inFlight[id] = struct{}{}
				}
				active++
				workers.Add(1)
				go func() {
					defer workers.Done()
					value, err := f(ctx, q)
					select {
					case answers <- response{id: id, value: codec.answer(id, value, err), releasesID: codec.rejectDuplicates}:
					case <-ctx.Done():
					}
				}()
			case commandwire.InputPull:
				pulls++
			case commandwire.Stderr:
				slog.Info("command service stream", "stream", codec.name, "message", string(r))
			case commandwire.Completed:
				return nil
			}
		}
	}
}
