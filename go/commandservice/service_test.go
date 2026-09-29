package commandservice_test

import (
	"bytes"
	"context"
	"errors"
	"io"
	"strings"
	"testing"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

func TestConcurrentBinaryEchoAndACancelKeepTheConnection(t *testing.T) {
	started, cancelled := newSignal(), newSignal()
	server := servicetest.Start(t, operations{
		"echo": echo,
		"wait": func(call *commandservice.Call) (commandservice.Completion, error) {
			started.send()
			<-call.Context().Done()
			cancelled.send()
			return commandservice.Completion{}, call.Context().Err()
		},
	})
	client := server.Client
	waiting, err := client.Invoke(testContext(t), invocation("wait"))
	if err != nil {
		t.Fatal(err)
	}
	started.receive(t)

	binary := []byte{0, 255, 13, 10, 128}
	got := run(t, client, invocation("echo"), binary)
	if !bytes.Equal(got.stdout, binary) || string(got.stderr) != "done" || got.completion.ExitCode != 7 {
		t.Errorf("echo = %q %q %+v", got.stdout, got.stderr, got.completion)
	}

	waiting.Cancel()
	cancelled.receive(t)
	if _, err := waiting.Next(); !errors.Is(err, commandservice.ErrCancelled) {
		t.Errorf("the cancelled stream's next record: error = %v, want ErrCancelled", err)
	}
	// The cancel did not take the connection with it.
	if _, err := client.Info(testContext(t)); err != nil {
		t.Errorf("the connection after a cancel: %v", err)
	}
}

func TestEachInputPullAsksForExactlyOneChunk(t *testing.T) {
	server := servicetest.Start(t, operations{"echo": echo})
	stream, err := server.Client.Invoke(testContext(t), invocation("echo"))
	if err != nil {
		t.Fatal(err)
	}
	next := func(want commandservice.RecordKind) commandservice.Record {
		t.Helper()
		record, err := stream.Next()
		if err != nil {
			t.Fatal(err)
		}
		if record.Kind != want {
			t.Fatalf("record kind %d, want %d", record.Kind, want)
		}
		return record
	}
	// A record is read only after the input it answers was sent, so a service
	// that asked for more than one chunk ahead would show as a second pull
	// where the echo belongs.
	for _, chunk := range []string{"first", "second"} {
		next(commandservice.RecordInputPull)
		if err := stream.Write([]byte(chunk)); err != nil {
			t.Fatal(err)
		}
		if got := next(commandservice.RecordStdout); string(got.Data) != chunk {
			t.Errorf("echo = %q, want %q", got.Data, chunk)
		}
	}
	next(commandservice.RecordInputPull)
	if err := stream.End(); err != nil {
		t.Fatal(err)
	}
	next(commandservice.RecordStderr)
	next(commandservice.RecordCompletion)
	if _, err := stream.Next(); err != io.EOF {
		t.Errorf("after the completion: error = %v, want io.EOF", err)
	}
}

func TestAResetInterruptsOutputBlockedOnFlowControl(t *testing.T) {
	ended := newSignal()
	server := servicetest.Start(t, operations{"flood": flood(ended)})
	stream, err := server.Client.Invoke(testContext(t), invocation("flood"))
	if err != nil {
		t.Fatal(err)
	}
	// Reading one record starts the flood; the rest fills the stream's window
	// and the output queue, and the handler blocks writing.
	if _, err := stream.Next(); err != nil {
		t.Fatal(err)
	}
	stream.Cancel()
	ended.receive(t)
}

func TestAnInvocationWithItsWholeInputInOneRequestBodyKeepsItsChunkBoundaries(t *testing.T) {
	// Metadata and input share the request body, and the input follows the
	// metadata without waiting for a pull: the service must keep the suffix.
	chunks := make(chan []byte, 4)
	server := servicetest.Start(t, operations{
		"chunks": func(call *commandservice.Call) (commandservice.Completion, error) {
			for {
				chunk, err := call.Stdin.Next()
				if err == io.EOF {
					close(chunks)
					return commandservice.Completion{}, nil
				}
				if err != nil {
					return commandservice.Completion{}, err
				}
				chunks <- chunk
			}
		},
	})
	stream, err := server.Client.Invoke(testContext(t), invocation("chunks"))
	if err != nil {
		t.Fatal(err)
	}
	// The whole input is written ahead of the pulls, by a goroutine, since a
	// write returns once the transport has taken it.
	go func() {
		for _, chunk := range [][]byte{{1}, {}, bytes.Repeat([]byte{2}, 64*1024)} {
			if err := stream.Write(chunk); err != nil {
				return
			}
		}
		_ = stream.End()
	}()
	var got [][]byte
	for chunk := range chunks {
		got = append(got, chunk)
		// Every chunk is a pull's answer; the pulls are answered by the writer.
		for {
			record, err := stream.Next()
			if err != nil || record.Kind == commandservice.RecordInputPull {
				break
			}
		}
	}
	if len(got) != 3 || len(got[0]) != 1 || len(got[1]) != 0 || len(got[2]) != 64*1024 {
		t.Errorf("chunk lengths %d, want 1, 0 and 65536", len(got))
	}
}

// A call that ends without a completion, because its handler returned a
// cancellation nobody asked for or because its completion is over the wire's
// limit, is reset, which the caller sees as an error; the service goes on.
func TestACallThatCannotSendItsCompletionIsResetAndKeepsTheService(t *testing.T) {
	server := servicetest.Start(t, operations{
		"cancelled": func(*commandservice.Call) (commandservice.Completion, error) {
			return commandservice.Completion{}, context.Canceled
		},
		"oversized": func(*commandservice.Call) (commandservice.Completion, error) {
			return commandservice.Completion{}, errors.New(strings.Repeat("x", 64*1024))
		},
		"short": short,
	})
	for _, operation := range []string{"cancelled", "oversized"} {
		stream, err := server.Client.Invoke(testContext(t), invocation(operation))
		if err != nil {
			t.Fatalf("%s: %v", operation, err)
		}
		if err := drain(stream); err == nil {
			t.Errorf("%s: the call ended with a completion", operation)
		}
	}
	if got := run(t, server.Client, invocation("short"), nil); string(got.stdout) != "ok" {
		t.Errorf("the call after them printed %q", got.stdout)
	}
}

// A handler's error reaches its caller as a completion with exit code 1 and the
// text of the error, whatever bytes the text holds: bytes that are not valid
// UTF-8 become U+FFFD, since JSON carries only UTF-8, and the call does not end
// without its completion for that.
func TestAHandlersErrorAlwaysReachesItsCallerAsACompletion(t *testing.T) {
	server := servicetest.Start(t, operations{
		"fails": func(*commandservice.Call) (commandservice.Completion, error) {
			return commandservice.Completion{}, errors.New("no such file")
		},
		"fails badly": func(*commandservice.Call) (commandservice.Completion, error) {
			return commandservice.Completion{}, errors.New("no such file \xff\xfe")
		},
		"says so badly": func(*commandservice.Call) (commandservice.Completion, error) {
			return commandservice.Completion{ExitCode: 2, Error: &commandservice.CommandError{Code: "custom", Message: "\xffgone"}}, nil
		},
	})
	for name, want := range map[string]commandservice.Completion{
		"fails":         {ExitCode: 1, Error: &commandservice.CommandError{Code: "command_failed", Message: "no such file"}},
		"fails badly":   {ExitCode: 1, Error: &commandservice.CommandError{Code: "command_failed", Message: "no such file \uFFFD"}},
		"says so badly": {ExitCode: 2, Error: &commandservice.CommandError{Code: "custom", Message: "\uFFFDgone"}},
	} {
		got := run(t, server.Client, invocation(name), nil).completion
		if got.ExitCode != want.ExitCode || got.Error == nil || *got.Error != *want.Error {
			t.Errorf("%s: completion %+v %+v, want %+v %+v", name, got, got.Error, want, want.Error)
		}
	}
}
