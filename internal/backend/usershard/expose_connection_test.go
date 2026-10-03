package usershard_test

import (
	"context"
	"errors"
	"net/netip"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

type netRunner struct {
	opens   chan *runnerwire.NetOpen
	answers chan []byte
}

func (r *netRunner) Receive(ctx context.Context) ([]byte, error) {
	select {
	case data := <-r.answers:
		return data, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func (r *netRunner) Send(ctx context.Context, data []byte) error {
	message, err := runnerwire.DecodeInbound(data)
	if err != nil {
		return err
	}
	if open, ok := message.(*runnerwire.NetOpen); ok {
		select {
		case r.opens <- open:
		case <-ctx.Done():
			return ctx.Err()
		}
	} else if _, ok := message.(*runnerwire.Ping); ok {
		return r.answer(ctx, &runnerwire.Pong{})
	}
	return nil
}

func (r *netRunner) answer(ctx context.Context, message runnerwire.Outbound) error {
	data, err := runnerwire.Encode(message)
	if err != nil {
		return err
	}
	select {
	case r.answers <- data:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func relayFixture(t *testing.T) (*fixture, *netRunner, webapi.ExposeID) {
	t.Helper()
	f := shardFixture(t, "relay")
	domain, err := expose.ParseDomain("expose.localhost")
	if err != nil {
		t.Fatal(err)
	}
	f.services.ExposeDomain = &domain
	f.services.PublicURL.Listening(nil, netip.MustParseAddrPort("127.0.0.1:3271"))
	device := f.devices[0]
	link, driver := remotehost.NewLink(
		remotehost.LinkOptions{
			Device:   string(device.ID),
			Identity: host.Identity{HomeDir: "/home/test"},
			Pipes:    f.shard.Pipes(),
		},
	)
	serving := f.shard.Devices().Bind(device.ID, link, driver, runners.NewLastSeen(f.services.Control, f.shard.Marks()))
	runner := &netRunner{
		opens:   make(chan *runnerwire.NetOpen, 2),
		answers: make(chan []byte, 2),
	}
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan struct{})
	go func() {
		defer close(done)
		serving.TestingServe(ctx, runner, runner)
	}()
	t.Cleanup(func() {
		cancel()
		<-done
	})
	record, err := expose.Add(t.Context(), f.shard.ExposeShard(), device.ID, "localhost:8080", time.Hour)
	if err != nil {
		t.Fatal(err)
	}
	return f, runner, record.Record.ID
}

type openedRelay struct {
	connection *usershard.ExposeConnection
	err        error
}

func requestRelay(
	ctx context.Context,
	t *testing.T,
	f *fixture,
	runner *netRunner,
	id webapi.ExposeID,
) (<-chan openedRelay, *runnerwire.NetOpen, []*remotehost.Pipe) {
	t.Helper()
	result := make(chan openedRelay, 1)
	go func() {
		connection, err := f.shard.OpenExposeConnection(ctx, id)
		result <- openedRelay{connection: connection, err: err}
	}()
	request := <-runner.opens
	if request.Host != "localhost" || request.Port != 8080 {
		t.Fatalf("network target = %s:%d", request.Host, request.Port)
	}
	var pipes []*remotehost.Pipe
	for _, ref := range []runnerwire.PipeRef{request.Input, request.Output} {
		pipe, ok := f.shard.Pipes().Pipe(ref.ID)
		if !ok {
			t.Fatal("network pipe missing before open reply")
		}
		pipes = append(pipes, pipe)
	}
	return result, request, pipes
}

// Cost: temporary SQLite and an in-process runner; virtual scheduling, no TCP service.
func TestExposeRelayEndsBothPipesAndReleasesAdmission(t *testing.T) {
	for _, ending := range []string{"edge", "removed", "shutdown"} {
		t.Run(ending, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				f, runner, id := relayFixture(t)
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				result, request, pipes := requestRelay(ctx, t, f, runner, id)
				if err := runner.answer(t.Context(), &runnerwire.NetOpened{StreamID: request.StreamID}); err != nil {
					t.Fatal(err)
				}
				opened := <-result
				if opened.err != nil {
					t.Fatal(opened.err)
				}
				lease := opened.connection.Lease
				defer lease.Release()
				// Request cancellation governs opening only; the returned lease owns copying.
				cancel()
				synctest.Wait()
				if err := lease.Context().Err(); err != nil {
					t.Fatalf("request canceled admitted relay: %v", err)
				}
				switch ending {
				case "edge":
					// Releasing one of two admissions must neither end nor leak the other.
					otherResult, otherRequest, _ := requestRelay(t.Context(), t, f, runner, id)
					if err := runner.answer(
						t.Context(),
						&runnerwire.NetOpened{StreamID: otherRequest.StreamID},
					); err != nil {
						t.Fatal(err)
					}
					other := <-otherResult
					if other.err != nil {
						t.Fatal(other.err)
					}
					defer other.connection.Lease.Release()
					lease.Release()
					synctest.Wait()
					if got := f.shard.ExposeShard().Exposes().Active(); got != 1 {
						t.Fatalf("admissions after first release = %d", got)
					}
					if err := other.connection.Lease.Context().Err(); err != nil {
						t.Fatalf("other lease ended: %v", err)
					}
					other.connection.Lease.Release()
				case "removed":
					if err := expose.Remove(t.Context(), f.shard.ExposeShard(), id); err != nil {
						t.Fatal(err)
					}
				case "shutdown":
					if err := f.shards.Close(t.Context()); err != nil {
						t.Fatal(err)
					}
				}
				<-lease.Context().Done()
				for _, pipe := range pipes {
					if err := pipe.Done(
						t.Context(),
					); err == nil ||
						err.Error() != "pipe failed: the relayed connection ended" {
						t.Fatalf("relay pipe end = %v", err)
					}
				}
				synctest.Wait()
				if got := f.shard.ExposeShard().Exposes().Active(); got != 0 {
					t.Fatalf("leaked admissions = %d", got)
				}
			})
		})
	}
}

func TestExposeOpenRefusalsReleaseAdmissionAndFailPipes(t *testing.T) {
	for _, refusal := range []string{
		"cancel",
		"removed",
		"offline",
		"unreachable",
	} {
		t.Run(refusal, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				f, runner, id := relayFixture(t)
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				result, request, pipes := requestRelay(ctx, t, f, runner, id)
				switch refusal {
				case "cancel":
					cancel()
				case "removed":
					if err := expose.Remove(t.Context(), f.shard.ExposeShard(), id); err != nil {
						t.Fatal(err)
					}
				case "offline":
					f.shard.Devices().DisconnectAll("test disconnected")
				case "unreachable":
					if err := runner.answer(
						t.Context(),
						&runnerwire.NetError{
							StreamID: request.StreamID,
							Code:     runnerwire.NetErrorCodeRefused,
							Message:  "connection refused",
						},
					); err != nil {
						t.Fatal(err)
					}
				}
				opened := <-result
				if opened.connection != nil {
					t.Fatal("failed open returned a connection")
				}
				if refusal == "unreachable" {
					var unreachable *expose.UnreachableError
					if !errors.As(opened.err, &unreachable) || unreachable.Code != "refused" {
						t.Fatalf("unreachable = %v", opened.err)
					}
				} else {
					want := expose.ErrRemoved
					if refusal == "offline" {
						want = expose.ErrRelayDeviceOffline
					}
					if !errors.Is(opened.err, want) {
						t.Fatalf("refusal = %v, want %v", opened.err, want)
					}
				}
				for _, pipe := range pipes {
					err := pipe.Done(t.Context())
					if !errors.Is(err, remotehost.ErrPipeFailed) {
						t.Fatalf("failed-open pipe = %v", err)
					}
					// Disconnect already fails every device pipe before OpenNet returns.
					if refusal != "offline" && err.Error() != "pipe failed: the relayed connection never opened" {
						t.Fatalf("failed-open pipe reason = %s", err)
					}
				}
				if got := f.shard.ExposeShard().Exposes().Active(); got != 0 {
					t.Fatalf("failed open leaked %d admissions", got)
				}
			})
		})
	}
}
