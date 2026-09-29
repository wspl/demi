package commandservice_test

import (
	"context"
	"errors"
	"io"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

func TestADrawTheWireRefusesIsRefusedBeforeAnyoneIsAsked(t *testing.T) {
	asked := 0
	numbers := commandservice.NewNumbers(func(context.Context, commandservice.NumbersRequest) (uint64, error) {
		asked++
		return 1, nil
	})
	for name, draw := range map[string]struct {
		conversation string
		sequence     commandservice.Sequence
		count        int
	}{
		"no numbers":          {"c", commandservice.SequenceTab, 0},
		"more than a request": {"c", commandservice.SequenceTab, 17},
		"no conversation":     {"", commandservice.SequenceTab, 1},
		"not a name":          {"a/b", commandservice.SequenceTab, 1},
		"another sequence":    {"c", "window", 1},
	} {
		_, err := numbers.Draw(testContext(t), draw.conversation, draw.sequence, draw.count)
		var invalid *commandservice.InvalidError
		if !errors.As(err, &invalid) {
			t.Errorf("%s: error = %v, want an InvalidError", name, err)
		}
	}
	if asked != 0 {
		t.Errorf("the numbers source was asked %d times for draws the wire refuses", asked)
	}
	if _, err := numbers.Draw(testContext(t), "c", commandservice.SequenceTab, 16); err != nil || asked != 1 {
		t.Errorf("a draw of the most a request holds: error %v, asked %d times", err, asked)
	}
}

// drawOnce draws one tab number of its conversation and prints it.
func drawOnce(call *commandservice.Call) (commandservice.Completion, error) {
	first, err := call.Numbers.Draw(call.Context(), call.Invocation.Context.Conversation, commandservice.SequenceTab, 1)
	if err != nil {
		return commandservice.Completion{}, err
	}
	_, err = call.Stdout.Write([]byte(strconv.FormatUint(first, 10)))
	return commandservice.Completion{}, err
}

func TestADrawTheRunnerRefusesFailsItsCallWithTheRunnersReasonAndLeavesTheStream(t *testing.T) {
	draws := make(chan error, 2)
	server := servicetest.Start(t, operations{"draw": func(call *commandservice.Call) (commandservice.Completion, error) {
		completion, err := drawOnce(call)
		draws <- err
		return completion, err
	}})
	stream, err := server.Client.Numbers(testContext(t))
	if err != nil {
		t.Fatal(err)
	}
	var asked atomic.Int32
	answered := make(chan error, 1)
	go func() {
		answered <- stream.Answer(testContext(t), func(context.Context, commandservice.NumbersRequest) (uint64, error) {
			if asked.Add(1) == 1 {
				return 0, errors.New("the conversation is gone")
			}
			return 5, nil
		})
	}()

	refused := run(t, server.Client, invocation("draw"), nil)
	err = <-draws
	if !errors.Is(err, commandservice.ErrNumbers) || !strings.Contains(err.Error(), "the conversation is gone") {
		t.Errorf("the refused draw failed with %v, want ErrNumbers with the runner's reason", err)
	}
	if refused.completion.ExitCode != 1 || refused.completion.Error == nil || !strings.Contains(refused.completion.Error.Message, "the conversation is gone") {
		t.Errorf("the call of the refused draw completed with %+v", refused.completion)
	}
	// A refusal answers one draw; the stream goes on.
	if next := run(t, server.Client, invocation("draw"), nil); next.completion.ExitCode != 0 || string(next.stdout) != "5" {
		t.Errorf("the next draw completed with %+v, printed %q", next.completion, next.stdout)
	}
	if err := <-draws; err != nil {
		t.Errorf("the next draw failed with %v", err)
	}
	if err := server.Client.Shutdown(testContext(t)); err != nil {
		t.Fatal(err)
	}
	if err := <-answered; err != nil {
		t.Errorf("the numbers stream ended with %v", err)
	}
}

// The text of a refusal can be any bytes; the runner's answer carries it as
// UTF-8, so bytes that are not become U+FFFD and the stream goes on.
func TestARefusalWhoseTextIsNotUTF8ReachesTheDrawWithReplacementsAndLeavesTheStream(t *testing.T) {
	server := servicetest.Start(t, operations{"draw": drawOnce})
	stream, err := server.Client.Numbers(testContext(t))
	if err != nil {
		t.Fatal(err)
	}
	answered := make(chan error, 1)
	go func() {
		answered <- stream.Answer(testContext(t), func(context.Context, commandservice.NumbersRequest) (uint64, error) {
			return 0, errors.New("gone \xff\xfe for good")
		})
	}()
	refused := run(t, server.Client, invocation("draw"), nil)
	if refused.completion.Error == nil || !strings.Contains(refused.completion.Error.Message, "gone \uFFFD for good") {
		t.Errorf("the call of the refused draw completed with %+v, want the reason with a replacement", refused.completion)
	}
	if err := server.Client.Shutdown(testContext(t)); err != nil {
		t.Fatal(err)
	}
	if err := <-answered; err != nil {
		t.Errorf("the numbers stream ended with %v", err)
	}
}

// The answering end sends what the source of the numbers returned; an answer
// that the wire would refuse, as one with an error that has no text, ends the
// answering.
func TestAnAnswerThatCannotBeEncodedEndsTheAnsweringWithItsError(t *testing.T) {
	server := servicetest.Start(t, operations{"draw": drawOnce})
	stream, err := server.Client.Numbers(testContext(t))
	if err != nil {
		t.Fatal(err)
	}
	answered := make(chan error, 1)
	go func() {
		answered <- stream.Answer(testContext(t), func(context.Context, commandservice.NumbersRequest) (uint64, error) {
			return 0, errors.New("")
		})
	}()
	// The draw is left without an answer, and fails when the stream ends.
	go func() { _, _ = drive(testContext(t), server.Client, invocation("draw")) }()
	var invalid *commandservice.InvalidError
	if err := <-answered; !errors.As(err, &invalid) {
		t.Errorf("the answering ended with %v, want an InvalidError", err)
	}
}

func TestADrawThatWaitsForTheStreamEndsWhenItsCallIsCancelled(t *testing.T) {
	started := newSignal()
	draws := make(chan error, 1)
	server := servicetest.Start(t, operations{"draw": func(call *commandservice.Call) (commandservice.Completion, error) {
		started.send()
		// No numbers stream is open, so the draw waits.
		_, err := call.Numbers.Draw(call.Context(), "conversation", commandservice.SequenceTab, 1)
		draws <- err
		return commandservice.Completion{}, err
	}})
	stream, err := server.Client.Invoke(testContext(t), invocation("draw"))
	if err != nil {
		t.Fatal(err)
	}
	started.receive(t)
	stream.Cancel()
	select {
	case err := <-draws:
		if !errors.Is(err, context.Canceled) {
			t.Errorf("the draw ended with %v, want the cancellation", err)
		}
	case <-time.After(hang):
		t.Fatal("the draw did not end with its call")
	}
}

// The runner opens the numbers stream once; a request for another is refused
// once its metadata is read.
func TestASecondNumbersStreamIsRefused(t *testing.T) {
	server := servicetest.Start(t, operations{"noop": short})
	if _, err := server.Client.Numbers(testContext(t)); err != nil {
		t.Fatal(err)
	}
	_, err := server.Client.Numbers(testContext(t))
	var rejected *commandservice.RejectedError
	if !errors.As(err, &rejected) || rejected.Status != 409 {
		t.Errorf("second stream: error = %v, want a rejection with status 409", err)
	}
}

// The answering end has at most 32 requests in flight: it refuses the 33rd at
// once, and does not ask the source of the numbers.
func TestTheAnsweringEndHoldsAtMost32RequestsInFlightAndRefusesTheNext(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		server := servicetest.Start(t, operations{"draw": drawOnce})
		stream, err := server.Client.Numbers(testContext(t))
		if err != nil {
			t.Fatal(err)
		}
		release := make(chan struct{})
		var asked atomic.Int32
		go func() {
			_ = stream.Answer(testContext(t), func(ctx context.Context, _ commandservice.NumbersRequest) (uint64, error) {
				asked.Add(1)
				select {
				case <-release:
					return 1, nil
				case <-ctx.Done():
					return 0, ctx.Err()
				}
			})
		}()
		type ended struct {
			completion commandservice.Completion
			err        error
		}
		completions := make(chan ended, 33)
		for range 33 {
			go func() {
				completion, err := drive(testContext(t), server.Client, invocation("draw"))
				completions <- ended{completion, err}
			}()
		}
		synctest.Wait()
		if got := asked.Load(); got != 32 {
			t.Errorf("the source of the numbers was asked %d times, want 32", got)
		}
		if len(completions) != 1 {
			t.Fatalf("%d draws ended while 32 wait for their numbers, want the 33rd alone", len(completions))
		}
		refused := <-completions
		if refused.err != nil || refused.completion.ExitCode != 1 || refused.completion.Error == nil ||
			!strings.Contains(refused.completion.Error.Message, "too many number requests in flight") {
			t.Errorf("the 33rd draw ended with %+v, %v", refused.completion, refused.err)
		}
		close(release)
		synctest.Wait()
		for range 32 {
			if answered := <-completions; answered.err != nil || answered.completion.ExitCode != 0 {
				t.Errorf("a draw that waited ended with %+v, %v", answered.completion, answered.err)
			}
		}
	})
}

// At most 64 draws wait for the numbers stream, whatever their callers do: the
// stream, opened later, carries a request for each of them, also of the callers
// that gave up, and holds back the rest.
func TestAtMost64DrawsWaitForTheNumbersStream(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		release := make(chan struct{})
		server := servicetest.Start(t, operations{"draws": func(call *commandservice.Call) (commandservice.Completion, error) {
			var drawing sync.WaitGroup
			var cancels []context.CancelFunc
			for range 100 {
				ctx, cancel := context.WithCancel(call.Context())
				cancels = append(cancels, cancel)
				drawing.Add(1)
				go func() {
					defer drawing.Done()
					_, _ = call.Numbers.Draw(ctx, "conversation", commandservice.SequenceTab, 1)
				}()
			}
			<-release
			for _, cancel := range cancels {
				cancel()
			}
			drawing.Wait()
			return commandservice.Completion{}, nil
		}})
		go func() { _, _ = drive(testContext(t), server.Client, invocation("draws")) }()
		// Every draw waits: 64 in the queue, and the others for room in it.
		synctest.Wait()
		close(release)
		synctest.Wait()

		numbers, err := server.Client.Numbers(testContext(t))
		if err != nil {
			t.Fatal(err)
		}
		var requests atomic.Int32
		go func() {
			for {
				record, err := numbers.Stream().Next()
				if err != nil {
					return
				}
				if record.Kind == commandservice.RecordStdout {
					requests.Add(1)
				}
			}
		}()
		synctest.Wait()
		if got := requests.Load(); got != 64 {
			t.Errorf("the numbers stream carried %d requests, want 64", got)
		}
	})
}

// drawing is a handler that draws four tab numbers of its conversation and
// prints the first, after it tells started.
type drawing struct {
	started chan struct{}
}

func (d *drawing) Operations() []string { return []string{"draw"} }

func (d *drawing) Invoke(call *commandservice.Call) (commandservice.Completion, error) {
	d.started <- struct{}{}
	first, err := call.Numbers.Draw(call.Context(), call.Invocation.Context.Conversation, commandservice.SequenceTab, 4)
	if err != nil {
		return commandservice.Completion{}, err
	}
	_, err = call.Stdout.Write([]byte(strconv.FormatUint(first, 10)))
	return commandservice.Completion{}, err
}

// The numbers stream as the runner sees it, record by record.
func TestTheNumbersStreamCarriesDrawsThatWaitedForItAndEndsWithTheRunnersInput(t *testing.T) {
	handler := &drawing{started: make(chan struct{}, 2)}
	server := servicetest.Start(t, handler)
	client := server.Client
	invoke := func() (*commandservice.Stream, error) { return client.Invoke(testContext(t), invocation("draw")) }

	// A draw made before the stream opens waits for it.
	early, err := invoke()
	if err != nil {
		t.Fatal(err)
	}
	<-handler.started
	numbers, err := client.Numbers(testContext(t))
	if err != nil {
		t.Fatal(err)
	}
	stream := numbers.Stream()
	var request commandservice.NumbersRequest
	pulled := false
	for request.Conversation == "" || !pulled {
		record, err := stream.Next()
		if err != nil {
			t.Fatal(err)
		}
		switch record.Kind {
		case commandservice.RecordStdout:
			if request, err = commandservice.Decode[commandservice.NumbersRequest](record.Data); err != nil {
				t.Fatal(err)
			}
		case commandservice.RecordInputPull:
			pulled = true
		}
	}
	want := commandservice.NumbersRequest{ID: 0, Conversation: "conversation", Sequence: commandservice.SequenceTab, Count: 4}
	if request != want {
		t.Errorf("request = %+v, want %+v", request, want)
	}
	first := uint64(9)
	answer, err := commandservice.Encode(commandservice.NumbersAnswer{ID: request.ID, First: &first})
	if err != nil {
		t.Fatal(err)
	}
	if err := stream.Write(answer); err != nil {
		t.Fatal(err)
	}
	var printed []byte
	for _, record := range readRecords(t, early) {
		if record.Kind == commandservice.RecordStdout {
			printed = append(printed, record.Data...)
		}
	}
	if string(printed) != "9" {
		t.Errorf("the draw printed %q, want the answer's 9", printed)
	}

	// The runner ends its input: the stream completes, and a draw made after
	// it fails.
	if err := stream.End(); err != nil {
		t.Fatal(err)
	}
	completed := false
	for {
		record, err := stream.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			t.Fatal(err)
		}
		completed = completed || (record.Kind == commandservice.RecordCompletion && record.Completion.ExitCode == 0)
	}
	if !completed {
		t.Error("the numbers stream ended without a completion of exit code 0")
	}
	late, err := invoke()
	if err != nil {
		t.Fatal(err)
	}
	var completion commandservice.Completion
	for _, record := range readRecords(t, late) {
		if record.Kind == commandservice.RecordCompletion {
			completion = record.Completion
		}
	}
	if completion.ExitCode != 1 || completion.Error == nil || !strings.Contains(completion.Error.Message, "numbers stream has ended") {
		t.Errorf("a draw after the stream ended completed with %+v", completion)
	}
}
