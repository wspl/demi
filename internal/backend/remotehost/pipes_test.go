package remotehost_test

import (
	"bytes"
	"context"
	"errors"
	"io"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(programTests{m}) }

// pipeBroker registers shutdown at acquisition for every pipe scenario.
func pipeBroker(t *testing.T, arrival time.Duration) *remotehost.Pipes {
	t.Helper()
	p := remotehost.NewPipes(arrival)
	t.Cleanup(func() {
		if err := p.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return p
}

// pipeBody supplies finite fixture bytes through the Host's cancellable stream boundary.
type pipeBody struct{ *bytes.Reader }

func (b pipeBody) Read(_ context.Context, p []byte) (int, error) { return b.Reader.Read(p) }
func (pipeBody) Close(context.Context) error                     { return nil }

// collectPipe observes pipe EOF or failure without treating a short stream as success.
func collectPipe(ctx context.Context, stream host.ByteStream) ([]byte, error) {
	var output []byte
	buf := make([]byte, 65536)
	for {
		n, err := stream.Read(ctx, buf)
		output = append(output, buf[:n]...)
		if errors.Is(err, io.EOF) {
			return output, nil
		}
		if err != nil {
			return output, err
		}
	}
}

// requirePipe checks scenario failures at their test boundary.
func requirePipe(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

// startUpload returns the owned upload's joined result.
func startUpload(ctx context.Context, source *remotehost.DeviceSource, body host.ByteStream) <-chan error {
	done := make(chan error, 1)
	go func() { done <- source.Pump(ctx, body) }()
	return done
}

// Cost: each scenario is in-process; virtual timer cases consume no wall time.
func TestDevicePutStreamsIntoDeviceGetAndAnswersOnceDrained(t *testing.T) {
	p := pipeBroker(t, 5*time.Second)
	pipe := p.Mint(new("a"), nil)
	requirePipe(t, pipe.SinkTo("b"))
	var fixed *remotehost.PipeError
	if !errors.As(pipe.SinkTo("a"), &fixed) || fixed.Kind != remotehost.PipeAlreadyFixed || fixed.End != "sink" {
		t.Fatal("sink was reassigned")
	}
	payload := make([]byte, 2*1024*1024)
	for i := range payload {
		payload[i] = byte(i)
	}
	sink, err := p.ClaimSink(pipe.ID(), "b")
	requirePipe(t, err)
	defer func() { requirePipe(t, sink.Close(context.Background())) }()
	source, err := p.ClaimSource(pipe.ID(), "a")
	requirePipe(t, err)
	done := startUpload(t.Context(), source, pipeBody{bytes.NewReader(payload)})
	requirePipe(t, sink.SourceArrived(t.Context()))
	got, err := collectPipe(t.Context(), sink)
	requirePipe(t, err)
	requirePipe(t, <-done)
	if !bytes.Equal(got, payload) {
		t.Fatal("stream changed")
	}
	requirePipe(t, pipe.Done(t.Context()))
	if _, err = p.ClaimSink(pipe.ID(), "b"); !errors.Is(err, remotehost.PipeNotFound) {
		t.Fatal(err)
	}
	second := p.Mint(new("a"), new("b"))
	if _, err = p.ClaimSink(second.ID(), "a"); !errors.Is(err, remotehost.PipeNotFound) {
		t.Fatal(err)
	}
	if _, err = p.ClaimSource(second.ID(), "b"); !errors.Is(err, remotehost.PipeNotFound) {
		t.Fatal(err)
	}
	claimed, err := p.ClaimSink(second.ID(), "b")
	requirePipe(t, err)
	defer func() { requirePipe(t, claimed.Close(context.Background())) }()
	if _, err = p.ClaimSink(second.ID(), "b"); !errors.Is(err, remotehost.PipeAlreadyConnected) {
		t.Fatal(err)
	}
	p.Fail(second.ID(), "test over")
	if err := second.Done(t.Context()); err == nil || err.Error() != "pipe failed: test over" {
		t.Fatal(err)
	}
}

func TestProcessReadsPutAndFeedsGetOneChunkAtATime(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		p := pipeBroker(t, 5*time.Second)
		inbound := p.FromDevice("a")
		reader, err := inbound.Reader()
		requirePipe(t, err)
		source, err := p.ClaimSource(inbound.ID(), "a")
		requirePipe(t, err)
		uploaded := startUpload(t.Context(), source, pipeBody{bytes.NewReader([]byte("hello, world"))})
		got, err := collectPipe(t.Context(), reader)
		requirePipe(t, err)
		requirePipe(t, reader.Close(t.Context()))
		requirePipe(t, <-uploaded)
		requirePipe(t, inbound.Done(t.Context()))
		if string(got) != "hello, world" {
			t.Fatal(string(got))
		}
		outbound := p.ToDevice("b")
		writer, err := outbound.Writer()
		requirePipe(t, err)
		progress := make(chan string, 3)
		done := make(chan error, 1)
		go func() {
			defer writer.Fail("the writer went away before the end")
			for _, word := range []string{"one ", "two ", "three"} {
				if err := writer.Write(t.Context(), []byte(word)); err != nil {
					done <- err
					return
				}
				progress <- word
			}
			writer.End()
			done <- nil
		}()
		synctest.Wait()
		if len(progress) != 1 {
			t.Fatalf("queued %d chunks without reader", len(progress))
		}
		sink, err := p.ClaimSink(outbound.ID(), "b")
		requirePipe(t, err)
		got, err = collectPipe(t.Context(), sink)
		requirePipe(t, err)
		requirePipe(t, sink.Close(t.Context()))
		requirePipe(t, <-done)
		requirePipe(t, outbound.Done(t.Context()))
		if string(got) != "one two three" {
			t.Fatal(string(got))
		}
		local := p.Mint(nil, nil)
		w, err := local.Writer()
		requirePipe(t, err)
		r, err := local.Reader()
		requirePipe(t, err)
		requirePipe(t, w.Write(t.Context(), []byte("local")))
		w.End()
		got, err = collectPipe(t.Context(), r)
		requirePipe(t, err)
		requirePipe(t, r.Close(t.Context()))
		if string(got) != "local" {
			t.Fatal(string(got))
		}
	})
}

func TestMissingEndTimesOutLostDeviceFailsAndEarlyReaderDrains(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		p := pipeBroker(t, 200*time.Millisecond)
		lonely := p.Mint(new("a"), new("b"))
		start := time.Now()
		err := lonely.Done(t.Context())
		if err == nil || !strings.Contains(err.Error(), "an end never arrived") ||
			time.Since(start) != 200*time.Millisecond {
			t.Fatal(err, time.Since(start))
		}
		if _, err := p.ClaimSource(lonely.ID(), "a"); !errors.Is(err, remotehost.PipeNotFound) {
			t.Fatal(err)
		}
		dropped := p.Mint(new("a"), new("b"))
		sink, err := p.ClaimSink(dropped.ID(), "b")
		requirePipe(t, err)
		p.DeviceGone("a")
		if err := sink.SourceArrived(
			t.Context(),
		); err == nil ||
			!strings.Contains(err.Error(), "device a disconnected") {
			t.Fatal(err)
		}
		requirePipe(t, sink.Close(t.Context()))
		early := p.FromDevice("a")
		reader, err := early.Reader()
		requirePipe(t, err)
		source, err := p.ClaimSource(early.ID(), "a")
		requirePipe(t, err)
		done := startUpload(t.Context(), source, pipeBody{bytes.NewReader(make([]byte, 4*1024*1024))})
		if b, err := reader.Next(t.Context()); err != nil || len(b) == 0 {
			t.Fatal(err)
		}
		requirePipe(t, reader.Close(t.Context()))
		requirePipe(t, <-done)
		requirePipe(t, early.Done(t.Context()))
	})
}

func TestArrivalWindowEndsOnceBothEndsArriveHoweverQuiet(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		p := pipeBroker(t, 40*time.Millisecond)
		pipe := p.Mint(nil, nil)
		w, err := pipe.Writer()
		requirePipe(t, err)
		r, err := pipe.Reader()
		requirePipe(t, err)
		time.Sleep(80 * time.Millisecond) // Virtual time verifies expiry is disarmed.
		requirePipe(t, w.Write(t.Context(), []byte("late")))
		w.End()
		got, err := collectPipe(t.Context(), r)
		requirePipe(t, err)
		requirePipe(t, r.Close(t.Context()))
		if string(got) != "late" {
			t.Fatal(string(got))
		}
		requirePipe(t, pipe.Done(t.Context()))
		device := p.Mint(new("a"), new("b"))
		source, err := p.ClaimSource(device.ID(), "a")
		requirePipe(t, err)
		sink, err := p.ClaimSink(device.ID(), "b")
		requirePipe(t, err)
		if _, err = p.ClaimSource(device.ID(), "a"); !errors.Is(err, remotehost.PipeAlreadyConnected) {
			t.Fatal(err)
		}
		if _, err = p.ClaimSink(device.ID(), "b"); !errors.Is(err, remotehost.PipeAlreadyConnected) {
			t.Fatal(err)
		}
		body := &gatedPipeBody{
			pipeBody: pipeBody{bytes.NewReader([]byte("late"))},
			entered:  make(chan struct{}),
			release:  make(chan struct{}),
		}
		done := startUpload(t.Context(), source, body)
		<-body.entered
		time.Sleep(80 * time.Millisecond)
		close(body.release)
		got, err = collectPipe(t.Context(), sink)
		requirePipe(t, err)
		requirePipe(t, sink.Close(t.Context()))
		requirePipe(t, <-done)
		if string(got) != "late" {
			t.Fatal(string(got))
		}
		requirePipe(t, device.Done(t.Context()))
	})
}

func TestHeldSourceHandedToDeviceWaitsForDevice(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		p := pipeBroker(t, 40*time.Millisecond)
		pipe := p.ToDevice("b")
		requirePipe(t, pipe.HoldSource())
		sink, err := p.ClaimSink(pipe.ID(), "b")
		requirePipe(t, err)
		requirePipe(t, pipe.SourceFrom("a"))
		if err = pipe.Done(t.Context()); err == nil || !strings.Contains(err.Error(), "an end never arrived") {
			t.Fatal(err)
		}
		if err = sink.SourceArrived(t.Context()); err == nil {
			t.Fatal("missing source arrived")
		}
		requirePipe(t, sink.Close(t.Context()))
		var failure *remotehost.PipeError
		if _, err = pipe.Writer(); !errors.As(err, &failure) || failure.Kind != remotehost.PipeSettled {
			t.Fatal(err)
		}
	})
}

// quietPipeBody waits on cancellation, exposing a quiet HTTP upload deterministically.
type quietPipeBody struct{ entered chan struct{} }

func (b quietPipeBody) Read(ctx context.Context, _ []byte) (int, error) {
	close(b.entered)
	<-ctx.Done()
	return 0, ctx.Err()
}
func (quietPipeBody) Close(context.Context) error { return nil }

func TestFailureInterruptsQuietBodyAndWaitingRead(t *testing.T) {
	p := pipeBroker(t, 5*time.Second)
	pipe := p.Mint(new("a"), new("b"))
	source, err := p.ClaimSource(pipe.ID(), "a")
	requirePipe(t, err)
	sink, err := p.ClaimSink(pipe.ID(), "b")
	requirePipe(t, err)
	body := quietPipeBody{make(chan struct{})}
	uploaded := startUpload(t.Context(), source, body)
	read := make(chan error, 1)
	go func() {
		_, err := sink.Next(t.Context())
		read <- err
	}()
	<-body.entered
	pipe.Fail("cancelled")
	for _, err := range []error{<-read, <-uploaded, pipe.Done(t.Context())} {
		if err == nil || err.Error() != "pipe failed: cancelled" {
			t.Fatal(err)
		}
	}
	requirePipe(t, sink.Close(t.Context()))
}

func TestDeviceReportCountsOnlyFromAnEndWhileOpen(t *testing.T) {
	p := pipeBroker(t, 5*time.Second)
	pipe := p.Mint(new("a"), new("b"))
	if p.FailFromDevice(pipe.ID(), "unrelated", "unauthorized failure") {
		t.Fatal("unrelated device ended pipe")
	}
	sink, err := p.ClaimSink(pipe.ID(), "b")
	requirePipe(t, err)
	if !p.FailFromDevice(pipe.ID(), "a", "upload refused") {
		t.Fatal("source failure ignored")
	}
	if p.FailFromDevice(pipe.ID(), "b", "later failure") {
		t.Fatal("terminal cause replaced")
	}
	if err := pipe.Done(t.Context()); err == nil || err.Error() != "pipe failed: upload refused" {
		t.Fatal(err)
	}
	if err := sink.SourceArrived(t.Context()); err == nil || !strings.Contains(err.Error(), "upload refused") {
		t.Fatal(err)
	}
	requirePipe(t, sink.Close(t.Context()))
}

func TestEmptyUploadSettlesBothEnds(t *testing.T) {
	p := pipeBroker(t, 5*time.Second)
	pipe := p.Mint(new("a"), new("b"))
	source, err := p.ClaimSource(pipe.ID(), "a")
	requirePipe(t, err)
	done := startUpload(t.Context(), source, pipeBody{bytes.NewReader(nil)})
	sink, err := p.ClaimSink(pipe.ID(), "b")
	requirePipe(t, err)
	got, err := collectPipe(t.Context(), sink)
	requirePipe(t, err)
	requirePipe(t, sink.Close(t.Context()))
	requirePipe(t, <-done)
	requirePipe(t, pipe.Done(t.Context()))
	if len(got) != 0 {
		t.Fatal(got)
	}
}

func TestEndLeavingBeforeEOFFailsPipe(t *testing.T) {
	p := pipeBroker(t, 5*time.Second)
	for _, response := range []bool{false, true} {
		pipe := p.ToDevice("b")
		sink, err := p.ClaimSink(pipe.ID(), "b")
		requirePipe(t, err)
		expected := "pipe failed: sink HTTP request disconnected"
		if response {
			writer, err := pipe.Writer()
			requirePipe(t, err)
			requirePipe(t, writer.Write(t.Context(), []byte("first")))
			first, err := sink.Next(t.Context())
			requirePipe(t, err)
			if string(first) != "first" {
				t.Fatal(string(first))
			}
			requirePipe(t, writer.Write(t.Context(), []byte("second")))
			defer writer.Fail("test over")
			expected = "pipe failed: sink HTTP response disconnected before EOF"
		}
		requirePipe(t, sink.Close(t.Context()))
		if err = pipe.Done(t.Context()); err == nil || err.Error() != expected {
			t.Fatal(err)
		}
	}
	pipe := p.FromDevice("a")
	source, err := p.ClaimSource(pipe.ID(), "a")
	requirePipe(t, err)
	ctx, cancel := context.WithCancel(t.Context())
	body := quietPipeBody{make(chan struct{})}
	done := startUpload(ctx, source, body)
	<-body.entered
	cancel()
	if err := <-done; err == nil || err.Error() != "pipe failed: source HTTP request disconnected" {
		t.Fatal(err)
	}
	if err := pipe.Done(t.Context()); err == nil || err.Error() != "pipe failed: source HTTP request disconnected" {
		t.Fatal(err)
	}
	local := p.Mint(nil, nil)
	w, err := local.Writer()
	requirePipe(t, err)
	r, err := local.Reader()
	requirePipe(t, err)
	w.Fail("the writer went away before the end")
	if _, err = r.Next(t.Context()); err == nil {
		t.Fatal("cut stream became EOF")
	}
	requirePipe(t, r.Close(t.Context()))
}

func TestClosingFailsEveryPipeAndEveryLaterOne(t *testing.T) {
	p := pipeBroker(t, 5*time.Second)
	open := p.FromDevice("a")
	requirePipe(t, p.Close(t.Context()))
	late := p.ToDevice("b")
	for _, pipe := range []*remotehost.Pipe{open, late} {
		if err := pipe.Done(t.Context()); err == nil || err.Error() != "pipe failed: backend shutting down" {
			t.Fatal(err)
		}
	}
}

// programTests releases compiled runner fixtures before checking goroutine ownership.
type programTests struct{ m *testing.M }

func (p programTests) Run() int { return programtest.Run(p.m) }

// gatedPipeBody holds a claimed device upload quiet before releasing its payload.
type gatedPipeBody struct {
	pipeBody
	entered chan struct{}
	release chan struct{}
	waiting bool
}

func (b *gatedPipeBody) Read(ctx context.Context, p []byte) (int, error) {
	if !b.waiting {
		b.waiting = true
		close(b.entered)
	}
	select {
	case <-ctx.Done():
		return 0, ctx.Err()
	case <-b.release:
		return b.pipeBody.Read(ctx, p)
	}
}
