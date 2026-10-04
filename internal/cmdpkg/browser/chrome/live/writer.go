package live

import (
	"context"
	"log/slog"
	"sync/atomic"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdsdk"
)

type writer struct {
	controls chan []byte
	videos   chan []byte
	queued   atomic.Int64
	done     chan struct{}
	cancel   context.CancelFunc
}

func startWriter(ctx context.Context, output *cmdsdk.Output) *writer {
	ctx, cancel := context.WithCancel(ctx)
	w := &writer{
		controls: make(chan []byte, 64),
		videos:   make(chan []byte, 4),
		done:     make(chan struct{}),
		cancel:   cancel,
	}
	go func() {
		defer close(w.done)
		w.run(ctx, output)
	}()
	return w
}

func (w *writer) control(ctx context.Context, message browserproto.LiveModuleMessage) {
	data, err := controlFrame(message)
	if err != nil {
		slog.Warn("live view message refused", "error", err)
		return
	}
	select {
	case <-ctx.Done():
	case <-w.done:
	case w.controls <- data:
	}
}

func (w *writer) notice(ctx context.Context, code, message string) {
	w.control(ctx, &browserproto.LiveModuleMessageNotice{Code: code, Message: message})
}

func (w *writer) video(data []byte) bool {
	w.queued.Add(int64(len(data)))
	select {
	case w.videos <- data:
		return true
	default:
		w.queued.Add(-int64(len(data)))
		return false
	}
}

// finish is called after all producers join, so closing the queues cannot race sends.
func (w *writer) finish(ctx context.Context) {
	close(w.controls)
	close(w.videos)
	timer := time.NewTimer(cdp.ControlTimeout)
	defer timer.Stop()
	select {
	case <-w.done:
	case <-ctx.Done():
		w.cancel()
	case <-timer.C:
		w.cancel()
	}
	w.cancel()
	<-w.done
}

func (w *writer) run(ctx context.Context, output *cmdsdk.Output) {
	heartbeat, err := controlFrame(&browserproto.LiveModuleMessageHeartbeat{})
	if err != nil {
		slog.Error("live heartbeat", "error", err)
		return
	}
	timer := time.NewTimer(time.Duration(browserproto.HeartbeatMS) * time.Millisecond)
	defer timer.Stop()
	for {
		var data []byte
		select {
		case data = <-w.controls:
			if data == nil {
				return
			}
		default:
			select {
			case <-ctx.Done():
				return
			case data = <-w.controls:
				if data == nil {
					return
				}
			case data = <-w.videos:
				if data == nil {
					return
				}
				w.queued.Add(-int64(len(data)))
			case <-timer.C:
				data = heartbeat
			}
		}
		if err := output.Stdout(ctx, data); err != nil {
			return
		}
		timer.Reset(time.Duration(browserproto.HeartbeatMS) * time.Millisecond)
	}
}
