package commandservice

import (
	"context"
	"fmt"
	"io"
	"log/slog"
	"strings"
	"sync"
	"sync/atomic"
	"unicode"
)

// numbersQueue is the most draws that wait for the numbers stream; a full queue
// holds back their callers.
const numbersQueue = 64

// maxNumbersInFlight is the most requests of one service that the answering
// end has in flight; it refuses one beyond at once.
const maxNumbersInFlight = 32

var errNumbersEnded = fmt.Errorf("%w: the numbers stream has ended", ErrNumbers)

// Numbers draws numbers of a conversation's sequences. Some state a service
// keeps is named with a number the model reads, which the conversation must
// never give twice, so the backend keeps the count and the runner forwards
// each request over the service's numbers stream. Numbers is safe for
// concurrent use.
type Numbers struct {
	reserve func(context.Context, NumbersRequest) (uint64, error)
}

// NewNumbers returns a numbers source that answers each draw by calling
// reserve, for a handler that runs without a numbers stream. The service's own
// source is the one its calls carry.
func NewNumbers(reserve func(ctx context.Context, request NumbersRequest) (uint64, error)) *Numbers {
	return &Numbers{reserve: reserve}
}

// Draw returns the first of count consecutive numbers of conversation's
// sequence, which now belong to the caller. A draw made before the runner
// opened the numbers stream waits for it. It fails when the runner refuses the
// request, when the stream ends, and when ctx ends. It refuses a request that
// breaks the wire's rules (an unnamed conversation, a count outside 1 to
// [MaxNumbers]) here, where the caller learns why, instead of where the runner
// reads it and breaks the stream.
func (n *Numbers) Draw(ctx context.Context, conversation string, sequence Sequence, count int) (uint64, error) {
	request := NumbersRequest{Conversation: conversation, Sequence: sequence, Count: count}
	if _, err := Encode(request); err != nil {
		return 0, err
	}
	return n.reserve(ctx, request)
}

// A draw is a request that waits for its answer.
type draw struct {
	request NumbersRequest
	answer  chan drawAnswer
}

type drawAnswer struct {
	first uint64
	err   error
}

// A numberSource is the connection's numbers source: draws wait in it for the
// runner's numbers stream, which writes each as a request record and hands
// back its answer. The runner opens the stream once. It ends at shutdown, when
// the runner ends its input, and when the service stops; a draw still waiting
// then fails.
type numberSource struct {
	draws   chan *draw
	claimed atomic.Bool

	// finishing is closed at shutdown: the stream completes.
	finishing  chan struct{}
	finishOnce sync.Once
	// ended is closed when the stream, or the service, has ended: no draw is
	// answered after.
	ended   chan struct{}
	endOnce sync.Once
}

func newNumberSource() *numberSource {
	return &numberSource{
		draws:     make(chan *draw, numbersQueue),
		finishing: make(chan struct{}),
		ended:     make(chan struct{}),
	}
}

// numbers returns the source as the handler sees it.
func (n *numberSource) numbers() *Numbers {
	return &Numbers{reserve: n.reserve}
}

// claim reports whether the caller may open the numbers stream: only the first
// request to ask does.
func (n *numberSource) claim() bool {
	return n.claimed.CompareAndSwap(false, true)
}

// finish tells the numbers stream to complete, as the service's shutdown does.
func (n *numberSource) finish() {
	n.finishOnce.Do(func() { close(n.finishing) })
}

// end fails every draw that is waiting, and every later one.
func (n *numberSource) end() {
	n.endOnce.Do(func() { close(n.ended) })
}

func (n *numberSource) reserve(ctx context.Context, request NumbersRequest) (uint64, error) {
	d := &draw{request: request, answer: make(chan drawAnswer, 1)}
	select {
	case n.draws <- d:
	case <-n.ended:
		return 0, errNumbersEnded
	case <-ctx.Done():
		return 0, ctx.Err()
	}
	select {
	case answer := <-d.answer:
		return answer.first, answer.err
	case <-n.ended:
		// The stream may have answered as it ended.
		select {
		case answer := <-d.answer:
			return answer.first, answer.err
		default:
			return 0, errNumbersEnded
		}
	case <-ctx.Done():
		return 0, ctx.Err()
	}
}

// relay serves the numbers stream: it writes each draw as one request record
// to the call's output and hands each answer, one input chunk, to the draw it
// answers. It returns when the service shuts down, when the runner ends the
// input, when a request or an answer breaks the wire, or when the call is
// cancelled.
func (n *numberSource) relay(state *callState) error {
	out := state.writer(RecordStdout)
	answers := make(chan NumbersAnswer)
	readEnd := make(chan error, 1)
	stop := make(chan struct{})
	readDone := make(chan struct{})
	go func() {
		defer close(readDone)
		readEnd <- readAnswers(&Input{state: state}, answers, stop)
	}()
	defer func() {
		close(stop)
		state.abort()
		<-readDone
		n.end()
	}()

	waiting := map[uint64]*draw{}
	var next uint64
	for {
		select {
		case d := <-n.draws:
			d.request.ID = next
			next++
			data, err := Encode(d.request)
			if err != nil {
				d.answer <- drawAnswer{err: err}
				continue
			}
			waiting[d.request.ID] = d
			if _, err := out.Write(data); err != nil {
				return err
			}
		case answer := <-answers:
			d, ok := waiting[answer.ID]
			if !ok {
				// The runner answered an ID this stream has no draw waiting
				// for.
				continue
			}
			delete(waiting, answer.ID)
			d.answer <- outcomeOf(answer)
		case err := <-readEnd:
			return err
		case <-n.finishing:
			return nil
		case <-state.ctx.Done():
			return ErrCancelled
		}
	}
}

// readAnswers reads the answers of the numbers stream, each one input chunk,
// until the runner ends its input (nil), the input fails, or stop is closed.
func readAnswers(in *Input, answers chan<- NumbersAnswer, stop <-chan struct{}) error {
	for {
		chunk, err := in.Next()
		if err == io.EOF {
			return nil
		}
		if err != nil {
			return err
		}
		answer, err := Decode[NumbersAnswer](chunk)
		if err != nil {
			return err
		}
		select {
		case answers <- answer:
		case <-stop:
			return nil
		}
	}
}

// outcomeOf returns the outcome of an answer that [Decode] has checked.
func outcomeOf(answer NumbersAnswer) drawAnswer {
	if answer.First != nil {
		return drawAnswer{first: *answer.First}
	}
	return drawAnswer{err: fmt.Errorf("%w: %s", ErrNumbers, *answer.Error)}
}

// A NumbersStream is a service's open numbers stream, which its caller
// answers.
type NumbersStream struct {
	stream *Stream
}

// numbersEvent is what the numbers stream's reader found next: a record, or
// the error that ended the stream.
type numbersEvent struct {
	record Record
	err    error
}

// Answer answers the service's requests until it ends the stream: each goes to
// reserve, at most 32 at a time, and each answer goes back as one input chunk
// when the service pulls. It returns nil when the service ends the stream, as
// its shutdown does, and cancels the stream and every reserve still running
// when it returns.
func (s *NumbersStream) Answer(ctx context.Context, reserve func(context.Context, NumbersRequest) (uint64, error)) error {
	ctx, cancel := context.WithCancel(ctx)
	events := make(chan numbersEvent)
	readDone := make(chan struct{})
	go func() {
		defer close(readDone)
		s.read(ctx, events)
	}()
	var reserving sync.WaitGroup
	defer func() {
		cancel()
		s.stream.Cancel()
		<-readDone
		reserving.Wait()
	}()

	reserved := make(chan NumbersAnswer)
	var queue []NumbersAnswer
	pulls := 0
	inFlight := 0
	for {
		// Each pull lets one answer go.
		for pulls > 0 && len(queue) > 0 {
			data, err := Encode(queue[0])
			if err != nil {
				return err
			}
			if err := s.stream.Write(data); err != nil {
				return err
			}
			queue = queue[1:]
			pulls--
		}
		select {
		case event := <-events:
			if event.err != nil {
				return event.err
			}
			switch event.record.Kind {
			case RecordStdout:
				request, err := Decode[NumbersRequest](event.record.Data)
				if err != nil {
					return err
				}
				if inFlight >= maxNumbersInFlight {
					refusal := "too many number requests in flight"
					queue = append(queue, NumbersAnswer{ID: request.ID, Error: &refusal})
					continue
				}
				inFlight++
				reserving.Add(1)
				go func() {
					defer reserving.Done()
					select {
					case reserved <- answerTo(ctx, request, reserve):
					case <-ctx.Done():
					}
				}()
			case RecordInputPull:
				pulls++
			case RecordStderr:
				slog.Info("numbers stream", "stderr", strings.TrimRightFunc(string(event.record.Data), unicode.IsSpace))
			case RecordCompletion:
				// The service ended the stream, as its shutdown does.
				return nil
			}
		case answer := <-reserved:
			inFlight--
			queue = append(queue, answer)
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

// read sends the records of the stream to events until it ends, or ctx does.
func (s *NumbersStream) read(ctx context.Context, events chan<- numbersEvent) {
	for {
		record, err := s.stream.Next()
		select {
		case events <- numbersEvent{record: record, err: err}:
		case <-ctx.Done():
			return
		}
		if err != nil || record.Kind == RecordCompletion {
			return
		}
	}
}

// answerTo asks reserve for the numbers of request and returns the answer
// that carries its outcome, as reserve gave it; bytes of a refusal's text that
// are not valid UTF-8 become U+FFFD.
func answerTo(ctx context.Context, request NumbersRequest, reserve func(context.Context, NumbersRequest) (uint64, error)) NumbersAnswer {
	first, err := reserve(ctx, request)
	if err != nil {
		// The text of an error can be any bytes, and JSON carries only UTF-8.
		message := strings.ToValidUTF8(err.Error(), "\uFFFD")
		return NumbersAnswer{ID: request.ID, Error: &message}
	}
	return NumbersAnswer{ID: request.ID, First: &first}
}
