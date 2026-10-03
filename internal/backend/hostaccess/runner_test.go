package hostaccess

import (
	"context"
	"errors"
	"io"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/backend/runners/runnerstest"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// boundRunner scripts a real connection through the existing Serving owner.
// Its channels carry generated wire frames, and runnerstest joins every worker.
type boundRunner struct {
	incoming   runnerFrames
	outgoing   runnerFrames
	connection *runnerstest.Connection
}

type runnerFrames chan []byte

func (f runnerFrames) Receive(ctx context.Context) ([]byte, error) {
	select {
	case frame := <-f:
		return frame, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}
func (f runnerFrames) Send(ctx context.Context, frame []byte) error {
	select {
	case f <- frame:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func connectHost(t *testing.T, s *testShard, device database.DeviceRecord) *boundRunner {
	t.Helper()
	link, driver := remotehost.NewLink(remotehost.LinkOptions{Device: string(device.ID), Identity: host.Identity{HomeDir: "/home/test", Hostname: "connected"}, Pipes: s.pipes, Policy: remotehosttest.NewCommandPolicy(nil)})
	serving := s.devices.Bind(device.ID, link, driver, runnerstest.LastSeen(s.control, s.owner))
	r := &boundRunner{incoming: make(runnerFrames, 64), outgoing: make(runnerFrames, 64)}
	r.connection = runnerstest.Serve(t, serving, r.incoming, r.outgoing)
	return r
}
func (r *boundRunner) next(t *testing.T) runnerwire.Inbound {
	t.Helper()
	frame, err := r.outgoing.Receive(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	message, err := runnerwire.DecodeInbound(frame)
	if err != nil {
		t.Fatal(err)
	}
	return message
}
func (r *boundRunner) send(t *testing.T, message runnerwire.Outbound) {
	t.Helper()
	frame, err := runnerwire.Encode(message)
	if err != nil {
		t.Fatal(err)
	}
	if err := r.incoming.Send(t.Context(), frame); err != nil {
		t.Fatal(err)
	}
}
func (r *boundRunner) stat(t *testing.T, size uint64) {
	t.Helper()
	request, ok := r.next(t).(*runnerwire.FSStat)
	if !ok {
		t.Fatal("expected stat")
	}
	r.send(t, &runnerwire.FSOK{ID: request.ID, Result: &runnerwire.FSStatResult{Value: runnerwire.FileStat{IsFile: true, Size: size, Mode: 0644, Mtime: 1767225600000}}})
}

type hostResult[T any] struct {
	value T
	err   error
}

// startHostOperation owns a concurrent request so failed assertions still cancel
// and join it before the shard and connection fixtures close.
func startHostOperation[T any](t *testing.T, operation func(context.Context) (T, error)) <-chan hostResult[T] {
	t.Helper()
	ctx, cancel := context.WithCancel(t.Context())
	results := make(chan hostResult[T], 1)
	done := make(chan struct{})
	t.Cleanup(func() {
		cancel()
		<-done
	})
	go func() {
		defer close(done)
		value, err := operation(ctx)
		results <- hostResult[T]{value: value, err: err}
	}()
	return results
}

// nextHostMessage checks the expected runner operation before a scripted reply.
func nextHostMessage[T runnerwire.Inbound](t *testing.T, r *boundRunner) T {
	t.Helper()
	message := r.next(t)
	wanted, ok := message.(T)
	if !ok {
		t.Fatalf("runner request: got %T, wanted %T", message, wanted)
	}
	return wanted
}

// receiveHostWrite drains the runner's incoming file bytes before acknowledging
// atomic replacement, so tests observe exactly what would be installed.
func receiveHostWrite(t *testing.T, s *testShard, r *boundRunner, device database.DeviceRecord) (string, []byte) {
	t.Helper()
	request := nextHostMessage[*runnerwire.FSWriteFile](t, r)
	sink, err := s.pipes.ClaimSink(request.Input.ID, string(device.ID))
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = sink.Close(context.Background()) }()
	var data []byte
	for {
		chunk, err := sink.Next(t.Context())
		data = append(data, chunk...)
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			t.Fatal(err)
		}
	}
	r.send(t, &runnerwire.FSOK{ID: request.ID, Result: &runnerwire.FSWriteFileResult{}})
	return request.Path, data
}

// sendHostRead gives the runner's requested output pipe its bytes and joins the
// pump with the test, including cancellation when a later assertion fails.
func sendHostRead(t *testing.T, s *testShard, r *boundRunner, device database.DeviceRecord, data string) string {
	t.Helper()
	request := nextHostMessage[*runnerwire.FSReadFile](t, r)
	source, err := s.pipes.ClaimSource(request.Output.ID, string(device.ID))
	if err != nil {
		t.Fatal(err)
	}
	// A pump failure reaches the operation through its pipe; test cleanup joins it.
	startHostOperation(t, func(ctx context.Context) (struct{}, error) {
		return struct{}{}, source.Pump(ctx, &hostBytes{Reader: strings.NewReader(data)})
	})
	r.send(t, &runnerwire.FSOK{ID: request.ID, Result: &runnerwire.FSReadFileResult{}})
	return request.Path
}

type hostBytes struct{ *strings.Reader }

func (b *hostBytes) Read(ctx context.Context, data []byte) (int, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	return b.Reader.Read(data)
}
func (*hostBytes) Close(context.Context) error { return nil }
