package commandservice

import (
	"context"
	"fmt"
	"io"
	"log/slog"
	"strings"
	"sync"
	"unicode"
)

type draw struct {
	request NumbersRequest
	answer  chan NumbersAnswer
}

// Numbers provides the service’s connection-scoped source of conversation numbers.
type Numbers struct {
	draws chan draw
	done  chan struct{}
	once  sync.Once
}

func newNumbers() *Numbers { return &Numbers{draws: make(chan draw, 64), done: make(chan struct{})} }

func (n *Numbers) stop() { n.once.Do(func() { close(n.done) }) }

// Draw reserves count numbers and returns their first number. Before the numbers stream
// opens it waits, until that stream answers or the context ends.
func (n *Numbers) Draw(ctx context.Context, conversation string, sequence Sequence, count int) (uint64, error) {
	req := NumbersRequest{Conversation: conversation, Sequence: sequence, Count: count}
	if _, err := Encode(req); err != nil {
		return 0, err
	}
	d := draw{request: req, answer: make(chan NumbersAnswer, 1)}
	select {
	case <-ctx.Done():
		return 0, ctx.Err()
	case <-n.done:
		return 0, fmt.Errorf("conversation numbers: the numbers stream has ended")
	case n.draws <- d:
	}
	select {
	case <-ctx.Done():
		return 0, ctx.Err()
	case <-n.done:
		return 0, fmt.Errorf("conversation numbers: the numbers stream has ended")
	case a := <-d.answer:
		if a.Error != nil {
			return 0, fmt.Errorf("conversation numbers: %s", *a.Error)
		}
		return *a.First, nil
	}
}

func (n *Numbers) relay(ctx context.Context, input *Input, body io.Closer) (Completion, error) {
	defer n.stop()
	ctx, cancel := context.WithCancel(ctx)
	queue := &recordOutput{ctx: ctx, records: input.output.records}
	input.output = queue
	output := recordWriter{output: queue, kind: Stdout}
	results := make(chan error, 2)
	var workers sync.WaitGroup
	var mu sync.Mutex
	waiting := make(map[uint64]chan NumbersAnswer)
	workers.Go(func() {
		var next uint64
		for {
			select {
			case <-ctx.Done():
				return
			case draw := <-n.draws:
				draw.request.ID = next
				next++
				mu.Lock()
				waiting[draw.request.ID] = draw.answer
				mu.Unlock()
				data, err := Encode(draw.request)
				if err != nil {
					results <- err
					return
				}
				if _, err = output.Write(data); err != nil {
					results <- err
					return
				}
			}
		}
	})
	workers.Go(func() {
		for {
			data, err := input.Next()
			if err != nil {
				results <- err
				return
			}
			answer, err := Decode[NumbersAnswer](data)
			if err != nil {
				results <- err
				return
			}
			mu.Lock()
			recipient := waiting[answer.ID]
			delete(waiting, answer.ID)
			mu.Unlock()
			if recipient != nil {
				recipient <- answer
			}
		}
	})
	defer func() {
		cancel()
		// Closing only interrupts a blocked read; its error cannot change the relay outcome.
		_ = body.Close()
		workers.Wait()
	}()
	select {
	case <-ctx.Done():
		return Completion{}, ctx.Err()
	case <-n.done:
		return Completion{}, nil
	case err := <-results:
		if err == io.EOF {
			return Completion{}, nil
		}
		return Completion{}, err
	}
}

// NumbersStream is the caller side of the service’s long-lived numbers request.
type NumbersStream struct{ stream *Stream }

// Answer answers at most 32 concurrent reservations, sends answers only on input demand,
// and returns on completion. The reserve callback must honor its context; callbacks are
// cancelled and joined on return.
func (s *NumbersStream) Answer(ctx context.Context, reserve func(context.Context, NumbersRequest) (uint64, error)) error {
	ctx, cancel := context.WithCancel(ctx)
	type received struct {
		record Record
		err    error
	}
	records := make(chan received)
	readerDone := make(chan struct{})
	go func() {
		defer close(readerDone)
		for {
			r, err := s.stream.Next()
			select {
			case records <- received{r, err}:
			case <-ctx.Done():
				return
			}
			if err != nil {
				return
			}
		}
	}()
	var workers sync.WaitGroup
	defer func() {
		cancel()
		s.stream.Cancel()
		<-readerDone
		workers.Wait()
	}()
	results := make(chan NumbersAnswer, 32)
	active := 0
	var pending []NumbersAnswer
	pulls := 0
	for {
		for pulls > 0 && len(pending) > 0 {
			b, err := Encode(pending[0])
			if err != nil {
				return err
			}
			if err = s.stream.Write(b); err != nil {
				return err
			}
			pending = pending[1:]
			pulls--
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case answer := <-results:
			active--
			pending = append(pending, answer)
		case item := <-records:
			if item.err != nil {
				if item.err == io.EOF {
					return nil
				}
				return item.err
			}
			switch item.record.Kind {
			case InputPull:
				pulls++
			case Completed:
				return nil
			case Stderr:
				slog.Info("numbers stream: " + strings.TrimRightFunc(strings.ToValidUTF8(string(item.record.Data), "�"), unicode.IsSpace))
			case Stdout:
				request, err := Decode[NumbersRequest](item.record.Data)
				if err != nil {
					return err
				}
				if active >= 32 {
					message := "too many number requests in flight"
					pending = append(pending, NumbersAnswer{ID: request.ID, Error: &message})
					continue
				}
				active++
				workers.Add(1)
				go func() {
					defer workers.Done()
					first, err := reserve(ctx, request)
					answer := NumbersAnswer{ID: request.ID, First: &first}
					if err != nil {
						message := err.Error()
						answer.First = nil
						answer.Error = &message
					}
					select {
					case results <- answer:
					case <-ctx.Done():
					}
				}()
			}
		}
	}
}
