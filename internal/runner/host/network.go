package host

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"strconv"
	"syscall"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runner/process"

	"github.com/wspl/demi/internal/runnerwire"
)

// NetOpen resolves the host on the device and connects with the Rust ten-second
// connection timeout. It answers net_opened before moving bytes, then joins both
// pipe directions. Input EOF half-closes the socket; socket EOF ends output;
// either pipe failing cancels both directions. It reports both pipe ends even
// when connecting fails. Streams do not consume finite Host-work permits.
func (s *Service) NetOpen(ctx context.Context, request runnerwire.NetOpen) error {
	ctx, leave, err := s.life.enter(ctx)
	if err != nil {
		return err
	}
	defer leave()
	socket, failure := connectTCP(ctx, request.Host, request.Port)
	if failure != nil {
		code := runnerwire.NetErrorCodeUnreachable
		var problem *netFailure
		if errors.As(failure, &problem) {
			code = problem.code
		}
		replyErr := sendFrame(s.life.ctx, s.output, &runnerwire.NetError{StreamID: request.StreamID, Code: code, Message: failure.Error()})
		inputErr := process.ReportPipe(s.life.ctx, s.output, request.Input.ID, failure)
		outputErr := process.ReportPipe(s.life.ctx, s.output, request.Output.ID, failure)
		return errors.Join(replyErr, inputErr, outputErr)
	}
	defer func() { _ = socket.Close() }() // Each direction reports its own IO failure.
	if err = sendFrame(s.life.ctx, s.output, &runnerwire.NetOpened{StreamID: request.StreamID}); err != nil {
		return err
	}
	exchange, cancel := context.WithCancel(ctx)
	defer cancel()
	closed := make(chan struct{})
	stop := context.AfterFunc(exchange, func() {
		_ = socket.Close() // Cancellation is already the failure; closing interrupts both directions.
		close(closed)
	})
	defer func() {
		if !stop() {
			<-closed
		}
	}()
	inputDone := make(chan error, 1)
	go func() {
		failure := s.netInput(exchange, request.Input.URL, socket)
		if failure != nil {
			cancel()
		}
		inputDone <- process.ReportPipe(s.life.ctx, s.output, request.Input.ID, failure)
	}()
	outputFailure := s.pipes.Put(exchange, request.Output.URL, &socketReader{socket})
	if outputFailure != nil {
		cancel()
	}
	outputErr := process.ReportPipe(s.life.ctx, s.output, request.Output.ID, outputFailure)
	inputErr := <-inputDone
	return errors.Join(inputErr, outputErr)
}

// netFailure retains socket error causes for descriptor retry and wire classification.
type netFailure struct {
	code    runnerwire.NetErrorCode
	message string
	cause   error
}

func (e *netFailure) Error() string { return e.message }
func (e *netFailure) Unwrap() error { return e.cause }

// connectTCP gives each socket attempt ten seconds, excluding descriptor backoff.
func connectTCP(ctx context.Context, host string, port uint16) (*net.TCPConn, error) {
	return cmdsdk.Retry(ctx, func() (*net.TCPConn, error) {
		attempt, cancel := context.WithTimeout(ctx, 10*time.Second)
		defer cancel()
		socket, err := (&net.Dialer{}).DialContext(attempt, "tcp", net.JoinHostPort(host, strconv.Itoa(int(port))))
		if err == nil {
			return socket.(*net.TCPConn), nil
		} // A successful tcp dial is a TCPConn.
		code := runnerwire.NetErrorCodeUnreachable
		message := err.Error()
		var dns *net.DNSError
		if errors.Is(attempt.Err(), context.DeadlineExceeded) {
			code = runnerwire.NetErrorCodeTimeout
			message = fmt.Sprintf("connecting to %s:%d exceeded 10 seconds", host, port)
		} else if errors.As(err, &dns) {
			code = runnerwire.NetErrorCodeResolveFailed
		} else if errors.Is(err, syscall.ECONNREFUSED) {
			code = runnerwire.NetErrorCodeRefused
		}
		return nil, &netFailure{code: code, message: message, cause: err}
	})
}

// netInput half-closes only the socket's write side on clean pipe EOF.
func (s *Service) netInput(ctx context.Context, url string, socket *net.TCPConn) (err error) {
	body, err := s.pipes.Open(ctx, url)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, body.Close()) }()
	if _, err = io.CopyBuffer(socket, body, make([]byte, 64*1024)); err != nil {
		return err
	}
	return socket.CloseWrite()
}

// socketReader transfers ownership of the TCP read half to the output pipe.
type socketReader struct{ *net.TCPConn }

func (r *socketReader) Close() error { return r.CloseRead() }
