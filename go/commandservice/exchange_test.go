package commandservice_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"strings"
	"testing"
	"testing/iotest"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// sizes prints the length of each chunk of its input, one to a line.
func sizes(call *commandservice.Call) (commandservice.Completion, error) {
	for {
		chunk, err := call.Stdin.Next()
		if err == io.EOF {
			return commandservice.Completion{}, nil
		}
		if err != nil {
			return commandservice.Completion{}, err
		}
		if _, err := fmt.Fprintf(call.Stdout, "%d\n", len(chunk)); err != nil {
			return commandservice.Completion{}, err
		}
	}
}

func TestEachPullIsAnsweredWithOneReadAndAShortReadIsOneChunk(t *testing.T) {
	server := servicetest.Start(t, operations{"sizes": sizes})
	for name, test := range map[string]struct {
		input io.Reader
		want  string
	}{
		"a short read":       {iotest.OneByteReader(strings.NewReader("abc")), "1\n1\n1\n"},
		"more than a record": {bytes.NewReader(make([]byte, 3*65536+8)), "65536\n65536\n65536\n8\n"},
		"no input":           {strings.NewReader(""), ""},
		"data with its end":  {iotest.DataErrReader(strings.NewReader("abcd")), "4\n"},
	} {
		stream, err := server.Client.Invoke(testContext(t), invocation("sizes"))
		if err != nil {
			t.Fatal(err)
		}
		var stdout bytes.Buffer
		if _, err := commandservice.Exchange(testContext(t), stream, test.input, &stdout, io.Discard); err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if stdout.String() != test.want {
			t.Errorf("%s: chunks %q, want %q", name, stdout.String(), test.want)
		}
	}
}

// noInput is an input that has nothing to give.
func noInput() io.Reader { return bytes.NewReader(nil) }

var errBroken = errors.New("the terminal is gone")

// failingWriter fails its first write.
type failingWriter struct{}

func (failingWriter) Write([]byte) (int, error) { return 0, errBroken }

func TestAnExchangeSaysWhichSideFailedAndCancelsTheInvocation(t *testing.T) {
	cancelled := newSignal()
	// A handler that prints, asks for input and then waits for its cancellation.
	waiting := func(call *commandservice.Call) (commandservice.Completion, error) {
		if _, err := io.WriteString(call.Stdout, "output"); err != nil {
			return commandservice.Completion{}, err
		}
		if _, err := call.Stdin.Next(); err != nil {
			cancelled.send()
			return commandservice.Completion{}, err
		}
		<-call.Context().Done()
		cancelled.send()
		return commandservice.Completion{}, call.Context().Err()
	}
	server := servicetest.Start(t, operations{"waiting": waiting, "echo": echo})
	exchange := func(input io.Reader, stdout io.Writer) error {
		stream, err := server.Client.Invoke(testContext(t), invocation("waiting"))
		if err != nil {
			t.Fatal(err)
		}
		_, err = commandservice.Exchange(testContext(t), stream, input, stdout, io.Discard)
		return err
	}
	for name, test := range map[string]struct {
		input  io.Reader
		stdout io.Writer
		side   commandservice.Side
	}{
		"the input":  {iotest.ErrReader(errBroken), io.Discard, commandservice.InputSide},
		"the output": {strings.NewReader("input"), failingWriter{}, commandservice.OutputSide},
	} {
		err := exchange(test.input, test.stdout)
		var failed *commandservice.ExchangeError
		if !errors.As(err, &failed) || failed.Side != test.side || !errors.Is(err, errBroken) {
			t.Errorf("%s failing: error = %v, want an ExchangeError of side %d wrapping the failure", name, err, test.side)
		}
		// The failure cancelled the invocation, which the handler saw.
		cancelled.receive(t)
	}
}

func TestAnExchangeFailsOnTheServiceSideWhenTheConnectionBreaks(t *testing.T) {
	started := newSignal()
	server := servicetest.Start(t, operations{"hold": func(call *commandservice.Call) (commandservice.Completion, error) {
		started.send()
		<-call.Context().Done()
		return commandservice.Completion{}, call.Context().Err()
	}})
	stream, err := server.Client.Invoke(testContext(t), invocation("hold"))
	if err != nil {
		t.Fatal(err)
	}
	failure := make(chan error, 1)
	go func() {
		_, err := commandservice.Exchange(testContext(t), stream, noInput(), io.Discard, io.Discard)
		failure <- err
	}()
	started.receive(t)
	if err := server.Client.Close(); err != nil {
		t.Fatal(err)
	}
	var failed *commandservice.ExchangeError
	if err := <-failure; !errors.As(err, &failed) || failed.Side != commandservice.ServiceSide {
		t.Errorf("error = %v, want an ExchangeError of the service side", err)
	}
}

func TestAnExchangeEndsWithItsContext(t *testing.T) {
	started, cancelled := newSignal(), newSignal()
	server := servicetest.Start(t, operations{"hold": func(call *commandservice.Call) (commandservice.Completion, error) {
		started.send()
		<-call.Context().Done()
		cancelled.send()
		return commandservice.Completion{}, call.Context().Err()
	}})
	stream, err := server.Client.Invoke(testContext(t), invocation("hold"))
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(testContext(t))
	failure := make(chan error, 1)
	go func() {
		_, err := commandservice.Exchange(ctx, stream, noInput(), io.Discard, io.Discard)
		failure <- err
	}()
	started.receive(t)
	cancel()
	if err := <-failure; !errors.Is(err, context.Canceled) {
		t.Errorf("error = %v, want the context's", err)
	}
	cancelled.receive(t)
}

func silenceLog(t *testing.T) {
	t.Helper()
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(io.Discard, nil)))
	t.Cleanup(func() { slog.SetDefault(previous) })
}

func TestAPanickingHandlerFailsItsOwnCallAndNotTheService(t *testing.T) {
	silenceLog(t)
	server := servicetest.Start(t, operations{
		"panic": func(*commandservice.Call) (commandservice.Completion, error) { panic("the handler broke") },
		"short": short,
	})
	stream, err := server.Client.Invoke(testContext(t), invocation("panic"))
	if err != nil {
		t.Fatal(err)
	}
	// The call ends without a completion, which the reset of its stream says.
	if _, err := commandservice.Exchange(testContext(t), stream, noInput(), io.Discard, io.Discard); err == nil {
		t.Error("the call of a handler that panicked completed")
	}
	if got := run(t, server.Client, invocation("short"), nil); string(got.stdout) != "ok" {
		t.Errorf("the next call printed %q", got.stdout)
	}
}
