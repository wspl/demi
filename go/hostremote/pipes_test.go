package hostremote_test

import (
	"bytes"
	"context"
	"errors"
	"io"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/hostremote"
)

// Cost: local byte channels only. The fake clock advances after both ends have
// arrived, or after the deliberately missing end has begun waiting.
func TestPipesRendezvousBackpressureAndLifetime(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		pipes := hostremote.NewPipes(120 * time.Second)
		defer pipes.Close()
		p := pipes.Mint("source", "")
		if err := p.SinkTo("sink"); err != nil {
			t.Fatal(err)
		}
		if _, err := pipes.ClaimSource(p.ID(), "sink"); !errors.Is(err, hostremote.ErrPipeNotFound) {
			t.Fatal(err)
		}
		source, err := pipes.ClaimSource(p.ID(), "source")
		if err != nil {
			t.Fatal(err)
		}
		sink, err := pipes.ClaimSink(p.ID(), "sink")
		if err != nil {
			t.Fatal(err)
		}
		if _, err := pipes.ClaimSink(p.ID(), "sink"); !errors.Is(err, hostremote.ErrPipeConnected) {
			t.Fatal(err)
		}
		synctest.Wait()
		time.Sleep(240 * time.Second)
		payload := bytes.Repeat([]byte{0, 255, 17, 42}, 32768)
		sent := make(chan error, 1)
		go func() { sent <- source.Pump(t.Context(), io.NopCloser(bytes.NewReader(payload))) }()
		if err := sink.SourceArrived(t.Context()); err != nil {
			t.Fatal(err)
		}
		data, err := io.ReadAll(sink)
		if err != nil || !bytes.Equal(data, payload) {
			t.Fatalf("transfer: %d %v", len(data), err)
		}
		if err := <-sent; err != nil {
			t.Fatal(err)
		}
		if err := p.Done(t.Context()); err != nil {
			t.Fatal(err)
		}
		if _, err := pipes.ClaimSource(p.ID(), "source"); !errors.Is(err, hostremote.ErrPipeNotFound) {
			t.Fatal(err)
		}

		local := pipes.Mint("", "")
		writer, err := local.Writer()
		if err != nil {
			t.Fatal(err)
		}
		reader, err := local.Reader()
		if err != nil {
			t.Fatal(err)
		}
		if _, err := writer.Write([]byte("first")); err != nil {
			t.Fatal(err)
		}
		written := make(chan error, 1)
		go func() {
			_, err := writer.Write([]byte("second"))
			written <- err
		}()
		synctest.Wait()
		select {
		case <-written:
			t.Fatal("a second chunk bypassed backpressure")
		default:
		}
		if data, err := reader.Next(t.Context()); err != nil || string(data) != "first" {
			t.Fatal(string(data), err)
		}
		if err := <-written; err != nil {
			t.Fatal(err)
		}
		writer.Close()
		if data, err := io.ReadAll(reader); err != nil || string(data) != "second" {
			t.Fatal(string(data), err)
		}

		held := pipes.ToDevice("sink")
		if err := held.HoldSource(); err != nil {
			t.Fatal(err)
		}
		heldSink, err := pipes.ClaimSink(held.ID(), "sink")
		if err != nil {
			t.Fatal(err)
		}
		arrived := make(chan error, 1)
		go func() { arrived <- heldSink.SourceArrived(t.Context()) }()
		synctest.Wait()
		time.Sleep(240 * time.Second)
		select {
		case <-arrived:
			t.Fatal("held source counted as a writer")
		default:
		}
		if err := held.SourceFrom("other"); err != nil {
			t.Fatal(err)
		}
		synctest.Wait()
		time.Sleep(120 * time.Second)
		if err := <-arrived; err == nil || err.Error() != "pipe failed: an end never arrived" {
			t.Fatal(err)
		}
	})
}

// Cost: no process or timer expiry; body reads and close events drive failures.
func TestPipesFailureInterruptsBodiesAndEarlyReadersDrain(t *testing.T) {
	pipes := hostremote.NewPipes(10 * time.Minute)
	defer pipes.Close()
	for _, early := range []bool{true, false} {
		p := pipes.FromDevice("source")
		reader, err := p.Reader()
		if err != nil {
			t.Fatal(err)
		}
		source, err := pipes.ClaimSource(p.ID(), "source")
		if err != nil {
			t.Fatal(err)
		}
		body, feed := io.Pipe()
		sent := make(chan error, 1)
		go func() { sent <- source.Pump(t.Context(), body) }()
		if _, err := feed.Write([]byte("ready")); err != nil {
			t.Fatal(err)
		}
		if _, err := reader.Next(t.Context()); err != nil {
			t.Fatal(err)
		}
		if early {
			reader.Close()
		} else {
			pipes.DeviceGone("source")
		}
		err = <-sent
		feed.Close()
		if early && err != nil {
			t.Fatal(err)
		}
		if !early && (err == nil || !strings.Contains(err.Error(), "device source disconnected")) {
			t.Fatal(err)
		}
	}
	p := pipes.Mint("a", "b")
	if pipes.FailFromDevice(p.ID(), "outsider", "forged") {
		t.Fatal("outsider failed a pipe")
	}
	if !pipes.FailFromDevice(p.ID(), "a", "failure") {
		t.Fatal("owner could not fail pipe")
	}
	if err := p.Done(context.Background()); err == nil || err.Error() != "pipe failed: failure" {
		t.Fatal(err)
	}
}
