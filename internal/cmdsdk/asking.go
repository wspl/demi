package cmdsdk

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"sync"

	"github.com/wspl/demi/internal/cmdproto"
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

func (a *asker[Q, R]) close() {
	a.once.Do(func() {
		close(a.done)
	})
}

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
type Draw = Pending[cmdproto.NumbersRequest, uint64]

// Numbers draws from conversation sequences through the runner.
type Numbers struct {
	source *asker[cmdproto.NumbersRequest, uint64]
}

// NumbersChannel creates a bounded source and its answering endpoint.
func NumbersChannel() (*Numbers, <-chan Draw) {
	a := newAsker[cmdproto.NumbersRequest, uint64]()
	return &Numbers{source: a}, a.requests
}

// Close ends the source and wakes all callers.
func (n *Numbers) Close() {
	n.source.close()
}

// Draw reserves count consecutive numbers and returns the first.
func (n *Numbers) Draw(
	ctx context.Context,
	conversation string,
	sequence cmdproto.ServiceSequence,
	count uint32,
) (uint64, error) {
	q := cmdproto.NumbersRequest{Conversation: conversation, Sequence: sequence, Count: count}
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
type ArtifactPending = Pending[cmdproto.ArtifactRequest, cmdproto.ArtifactAnswer]

// Artifacts asks the runner to install or list artifacts, never deriving their paths.
type Artifacts struct {
	source *asker[cmdproto.ArtifactRequest, cmdproto.ArtifactAnswer]
}

// ArtifactsChannel creates a bounded artifact source and its answering endpoint.
func ArtifactsChannel() (*Artifacts, <-chan ArtifactPending) {
	a := newAsker[cmdproto.ArtifactRequest, cmdproto.ArtifactAnswer]()
	return &Artifacts{source: a}, a.requests
}

// Close ends the source and wakes callers.
func (a *Artifacts) Close() {
	a.source.close()
}

func (a *Artifacts) ask(ctx context.Context, q cmdproto.ArtifactRequest) (cmdproto.ArtifactAnswer, error) {
	if err := q.Validate(); err != nil {
		return cmdproto.ArtifactAnswer{}, err
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
func (a *Artifacts) Install(ctx context.Context, install cmdproto.ArtifactInstall) (string, error) {
	r, err := a.ask(ctx, cmdproto.ArtifactRequest{Install: &install})
	if err != nil {
		return "", err
	}
	if r.Path == nil {
		return "", errors.New("runner answered another kind of artifact request")
	}
	return *r.Path, nil
}

// Installed returns the Host's artifacts of a line, newest first.
func (a *Artifacts) Installed(ctx context.Context, name string) ([]cmdproto.InstalledArtifact, error) {
	r, err := a.ask(ctx, cmdproto.ArtifactRequest{Installed: &cmdproto.ArtifactsInstalled{Name: name}})
	if err != nil {
		return nil, err
	}
	if r.Installed == nil {
		return nil, errors.New("runner answered another kind of artifact request")
	}
	return *r.Installed, nil
}

type askCodec[Q json.Marshaler, R any, A json.Marshaler] struct {
	name             string
	rejectDuplicates bool
	request          func([]byte) (Q, error)
	identify         func(Q, uint64) Q
	id               func(Q) uint64
	answer           func(uint64, R, error) A
	decode           func([]byte) (A, error)
	outcome          func(A) (uint64, R, error)
}

var numberCodec = askCodec[cmdproto.NumbersRequest, uint64, cmdproto.NumbersAnswer]{
	rejectDuplicates: true,
	name:             "numbers",
	request:          cmdproto.DecodeNumbersRequest,
	identify: func(q cmdproto.NumbersRequest, id uint64) cmdproto.NumbersRequest {
		q.ID = id
		return q
	},
	id: func(q cmdproto.NumbersRequest) uint64 {
		return q.ID
	},
	answer: func(id, value uint64, err error) cmdproto.NumbersAnswer {
		r := cmdproto.NumbersAnswer{ID: id}
		if err != nil {
			s := err.Error()
			r.Error = &s
		} else {
			r.First = &value
		}
		return r
	},
	decode: cmdproto.DecodeNumbersAnswer,
	outcome: func(a cmdproto.NumbersAnswer) (uint64, uint64, error) {
		if a.Error != nil {
			return a.ID, 0, errors.New(*a.Error)
		}
		return a.ID, *a.First, nil
	},
}

var artifactCodec = askCodec[cmdproto.ArtifactRequest, cmdproto.ArtifactAnswer, cmdproto.ArtifactAnswer]{
	name: "artifacts", request: cmdproto.DecodeArtifactRequest,
	identify: func(q cmdproto.ArtifactRequest, id uint64) cmdproto.ArtifactRequest {
		q.ID = id
		return q
	},
	id: func(q cmdproto.ArtifactRequest) uint64 {
		return q.ID
	},
	answer: func(id uint64, value cmdproto.ArtifactAnswer, err error) cmdproto.ArtifactAnswer {
		value.ID = id
		if err != nil {
			s := err.Error()
			return cmdproto.ArtifactAnswer{ID: id, Error: &s}
		}
		return value
	},
	decode: cmdproto.DecodeArtifactAnswer,
	outcome: func(a cmdproto.ArtifactAnswer) (uint64, cmdproto.ArtifactAnswer, error) {
		if a.Error != nil {
			return a.ID, a, errors.New(*a.Error)
		}
		return a.ID, a, nil
	},
}

func relayNumbers(
	ctx, finish context.Context,
	i *Input,
	o *Output,
	n *Numbers,
	q <-chan Draw,
) (cmdproto.Completion, error) {
	return relay(ctx, finish, i, o, n.source, q, numberCodec)
}

func relayArtifacts(
	ctx, finish context.Context,
	i *Input,
	o *Output,
	a *Artifacts,
	q <-chan ArtifactPending,
) (cmdproto.Completion, error) {
	return relay(ctx, finish, i, o, a.source, q, artifactCodec)
}

// relay correlates requests on one service side stream with their answers.
func relay[Q json.Marshaler, R any, A json.Marshaler](
	ctx, finish context.Context,
	i *Input,
	o *Output,
	source *asker[Q, R],
	requests <-chan Pending[Q, R],
	codec askCodec[Q, R, A],
) (cmdproto.Completion, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	defer source.close()
	stop := context.AfterFunc(finish, cancel)
	defer stop()
	var mu sync.Mutex
	waiting := map[uint64]Pending[Q, R]{}
	done := make(chan error, 2)
	go func() {
		done <- sendRequests(ctx, source, requests, codec, o, &mu, waiting)
	}()
	go func() {
		done <- receiveAnswers(ctx, codec, i, &mu, waiting)
	}()
	err := <-done
	cancel()
	<-done
	if finish.Err() != nil || errors.Is(err, io.EOF) {
		err = nil
	}
	return cmdproto.Completion{}, err
}

// RequestStream is one runner endpoint answering the service's requests.
type RequestStream struct {
	input  *CommandInput
	output *CommandOutput
}

func (c *Client) requestStream(ctx context.Context, path string) (*RequestStream, error) {
	b, err := (cmdproto.StreamOpen{}).Encode()
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
	return c.requestStream(ctx, cmdproto.NumbersPath)
}

// Artifacts opens the service's single artifacts stream.
func (c *Client) Artifacts(ctx context.Context) (*RequestStream, error) {
	return c.requestStream(ctx, cmdproto.ArtifactsPath)
}

// AnswerNumbers answers at most 32 requests concurrently, refusing excess requests.
func (s *RequestStream) AnswerNumbers(
	ctx context.Context,
	f func(context.Context, cmdproto.NumbersRequest) (uint64, error),
) error {
	return answerStream(ctx, s, numberCodec, f)
}

// AnswerArtifacts answers the artifact stream with runner-owned installations.
func (s *RequestStream) AnswerArtifacts(
	ctx context.Context,
	f func(context.Context, cmdproto.ArtifactRequest) (cmdproto.ArtifactAnswer, error),
) error {
	return answerStream(ctx, s, artifactCodec, f)
}

// answerStream correlates runner callbacks with the service's input demands.
func answerStream[Q json.Marshaler, R any, A json.Marshaler](
	ctx context.Context,
	s *RequestStream,
	codec askCodec[Q, R, A],
	f func(context.Context, Q) (R, error),
) error {
	ctx, cancel := context.WithCancel(ctx)
	var workers sync.WaitGroup
	defer func() {
		cancel()
		s.input.Cancel()
		workers.Wait()
	}()
	records := make(chan cmdproto.Record)
	failures := make(chan error, 1)
	workers.Add(1)
	go func() {
		defer workers.Done()
		s.readRequests(ctx, records, failures)
	}()
	state := streamAnswers[A]{answers: make(chan streamResponse[A], 32), inFlight: make(map[uint64]struct{})}
	for {
		if err := state.writeQueued(ctx, s.input); err != nil {
			return err
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case err := <-failures:
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		case a := <-state.answers:
			state.active--
			state.queued = append(state.queued, a)
		case r := <-records:
			switch r := r.(type) {
			case cmdproto.Stdout:
				if err := answerRequest(ctx, r, codec, f, &state, &workers); err != nil {
					return err
				}
			case cmdproto.InputPull:
				state.pulls++
			case cmdproto.Stderr:
				slog.Info("command service stream", "stream", codec.name, "message", string(r))
			case cmdproto.Completed:
				return nil
			}
		}
	}
}

// sendRequests correlates service-stream requests with emitted IDs.
func sendRequests[Q json.Marshaler, R any, A json.Marshaler](
	ctx context.Context,
	source *asker[Q, R],
	requests <-chan Pending[Q, R],
	codec askCodec[Q, R, A],
	o *Output,
	mu *sync.Mutex,
	waiting map[uint64]Pending[Q, R],
) error {
	var next uint64
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-source.done:
			return nil
		case p := <-requests:
			id := next
			next++
			mu.Lock()
			waiting[id] = p
			mu.Unlock()
			b, err := codec.identify(p.Request, id).MarshalJSON()
			if err == nil {
				err = o.Stdout(ctx, b)
			}
			if err != nil {
				return err
			}
		}
	}
}

// receiveAnswers correlates service-stream answers with pending callers.
func receiveAnswers[Q json.Marshaler, R any, A json.Marshaler](
	ctx context.Context,
	codec askCodec[Q, R, A],
	i *Input,
	mu *sync.Mutex,
	waiting map[uint64]Pending[Q, R],
) error {
	for {
		b, err := i.Next(ctx)
		if err != nil {
			return err
		}
		a, err := codec.decode(b)
		if err != nil {
			return err
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
}

// streamResponse retains the ID until its original answer has been sent.
type streamResponse[A json.Marshaler] struct {
	id         uint64
	value      A
	releasesID bool
}

// streamAnswers bundles answer queue and admission state for one runner stream.
type streamAnswers[A json.Marshaler] struct {
	answers chan streamResponse[A]
	// IDs remain in flight until their original answer is sent, even after
	// its callback finishes. A duplicate refusal does not release that ID.
	inFlight      map[uint64]struct{}
	active, pulls int
	queued        []streamResponse[A]
}

// writeQueued sends queued answers only when the service requests input.
func (sa *streamAnswers[A]) writeQueued(ctx context.Context, input *CommandInput) error {
	for sa.pulls > 0 && len(sa.queued) > 0 {
		b, err := sa.queued[0].value.MarshalJSON()
		if err != nil {
			return err
		}
		if err = input.Write(ctx, b); err != nil {
			return err
		}
		if sa.queued[0].releasesID {
			delete(sa.inFlight, sa.queued[0].id)
		}
		sa.queued = sa.queued[1:]
		sa.pulls--
	}
	return nil
}

// answerRequest validates, admits or refuses one request before starting its callback.
func answerRequest[Q json.Marshaler, R any, A json.Marshaler](
	ctx context.Context,
	r cmdproto.Stdout,
	codec askCodec[Q, R, A],
	f func(context.Context, Q) (R, error),
	state *streamAnswers[A],
	workers *sync.WaitGroup,
) error {
	q, err := codec.request(r)
	if err != nil {
		return err
	}
	id := codec.id(q)
	if _, exists := state.inFlight[id]; codec.rejectDuplicates && exists {
		var zero R
		state.queued = append(
			state.queued,
			streamResponse[A]{
				value: codec.answer(id, zero, fmt.Errorf("duplicate %s request id %d", codec.name, id)),
			},
		)
		return nil
	}
	if state.active == 32 {
		var zero R
		state.queued = append(
			state.queued,
			streamResponse[A]{
				value: codec.answer(id, zero, fmt.Errorf("too many %s requests in flight", codec.name)),
			},
		)
		return nil
	}
	if codec.rejectDuplicates {
		state.inFlight[id] = struct{}{}
	}
	state.active++
	workers.Add(1)
	go func() {
		defer workers.Done()
		value, err := f(ctx, q)
		select {
		case state.answers <- streamResponse[A]{
			id:         id,
			value:      codec.answer(id, value, err),
			releasesID: codec.rejectDuplicates,
		}:
		case <-ctx.Done():
		}
	}()
	return nil
}

// readRequests forwards service records until the stream ends or its owner cancels.
func (s *RequestStream) readRequests(ctx context.Context, records chan<- cmdproto.Record, failures chan<- error) {
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
}
