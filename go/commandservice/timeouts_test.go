package commandservice_test

import (
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// The tests of a time limit run in a synctest bubble, where the clock is fake
// and moves to the next timer whenever every goroutine waits: each runs the
// wire's own limit, in no time, and asserts the time the event came: not before
// the limit, and not after it.

func TestMetadataThatDoesNotArriveInTimeIsRefused(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		server := servicetest.Start(t, operations{"noop": short})
		// The request stays open and sends nothing.
		reader, writer := io.Pipe()
		defer writer.Close()
		ctx, cancel := context.WithCancel(testContext(t))
		defer cancel()
		start := time.Now()
		_, err := server.Client.Send(ctx, http.MethodPost, commandservice.InvokePath, reader, 0)
		if httpStatus(err) != http.StatusBadRequest {
			t.Errorf("error = %v, want status 400", err)
		}
		if elapsed := time.Since(start); elapsed != 10*time.Second {
			t.Errorf("the request was refused after %v, want the 10 s it has", elapsed)
		}
	})
}

func TestAHandlerThatIgnoresItsCancellationFaultsTheServiceOnceTheGraceEnds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		started := newSignal()
		release := make(chan struct{})
		t.Cleanup(func() { close(release) })
		server := servicetest.Start(t, operations{"stubborn": func(*commandservice.Call) (commandservice.Completion, error) {
			started.send()
			<-release
			return commandservice.Completion{}, nil
		}})
		stream, err := server.Client.Invoke(testContext(t), invocation("stubborn"))
		if err != nil {
			t.Fatal(err)
		}
		started.receive(t)
		cancelled := time.Now()
		stream.Cancel()
		if err := server.Wait(testContext(t)); !errors.Is(err, commandservice.ErrCancellationDeadline) {
			t.Errorf("Serve returned %v, want ErrCancellationDeadline", err)
		}
		if elapsed := time.Since(cancelled); elapsed != 5*time.Second {
			t.Errorf("the service faulted after %v, want the grace of 5 s", elapsed)
		}
	})
}

// stuckClose is a handler whose Close never returns by itself.
type stuckClose struct {
	operations
	release chan struct{}
}

func (h stuckClose) Close(context.Context) error {
	<-h.release
	return nil
}

func TestAHandlerThatDoesNotCloseInTimeFaultsTheServiceOnceTheGraceEnds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		handler := stuckClose{operations: operations{"noop": short}, release: make(chan struct{})}
		t.Cleanup(func() { close(handler.release) })
		server := servicetest.Start(t, handler)
		if _, err := server.Client.Info(testContext(t)); err != nil {
			t.Fatal(err)
		}
		closing := time.Now()
		if err := server.Client.Close(); err != nil {
			t.Fatal(err)
		}
		if err := server.Wait(testContext(t)); !errors.Is(err, commandservice.ErrCancellationDeadline) {
			t.Errorf("Serve returned %v, want ErrCancellationDeadline", err)
		}
		if elapsed := time.Since(closing); elapsed != 5*time.Second {
			t.Errorf("the service faulted after %v, want the grace of 5 s", elapsed)
		}
	})
}

func TestAPeerThatNeverCompletesTheHandshakeFailsTheServiceOnceItsTimeEnds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		clientEnd, serviceEnd := net.Pipe()
		defer clientEnd.Close()
		start := time.Now()
		err := commandservice.Serve(testContext(t), serviceEnd, operations{"noop": short})
		if !errors.Is(err, commandservice.ErrHandshakeTimeout) {
			t.Errorf("Serve returned %v, want ErrHandshakeTimeout", err)
		}
		if elapsed := time.Since(start); elapsed != 10*time.Second {
			t.Errorf("the handshake timed out after %v, want the 10 s it has", elapsed)
		}
	})
}

// The time limit of the handshake ends with the handshake: a connection that
// went on to serve requests is not retired when it would have passed.
func TestACompletedHandshakeIsNoLongerTimed(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		server := servicetest.Start(t, operations{"noop": short})
		if _, err := server.Client.Info(testContext(t)); err != nil {
			t.Fatal(err)
		}
		time.Sleep(20 * time.Second)
		if _, err := server.Client.Info(testContext(t)); err != nil {
			t.Errorf("after the time of the handshake: %v", err)
		}
		if err := server.Client.Close(); err != nil {
			t.Fatal(err)
		}
		if err := server.Wait(testContext(t)); err != nil {
			t.Errorf("Serve returned %v", err)
		}
	})
}
