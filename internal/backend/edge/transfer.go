package edge

import (
	"context"
	"errors"
	"io"
	"net/http"
	"time"

	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
)

const transferIdle = 60 * time.Second

// bodyStream makes a request body's blocking reads honor each copy's context.
// The request owns the connection; deadlines wake reads without a detached worker.
type bodyStream struct {
	body    io.ReadCloser
	control *http.ResponseController
}

func (s *bodyStream) Read(ctx context.Context, p []byte) (int, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		defer close(done)
		_ = s.control.SetReadDeadline(time.Now())
	})
	n, err := s.body.Read(p)
	if !stop() {
		<-done
	}
	_ = s.control.SetReadDeadline(time.Time{})
	if ctx.Err() != nil {
		return n, ctx.Err()
	}
	return n, err
}
func (s *bodyStream) Close(context.Context) error { return s.body.Close() }

// copyUpload paces reads by the Host's acceptance, timing only one side at a
// time. Failure leaves the Host's atomic upload uncommitted.
func copyUpload(ctx context.Context, body host.ByteStream, writer *remotehost.PipeWriter, written func(context.Context) error) error {
	defer writer.Fail("the upload was cut short")
	buffer := make([]byte, 64*1024)
	changed := func() error { return apiFailure(409, "conversation_busy", "The conversation changed under the upload") }
	for {
		if ctx.Err() != nil {
			return changed()
		}
		reading, cancel := context.WithTimeout(ctx, transferIdle)
		n, err := body.Read(reading, buffer)
		timedOut := errors.Is(reading.Err(), context.DeadlineExceeded)
		cancel()
		if ctx.Err() != nil {
			return changed()
		}
		if timedOut {
			writer.Fail("the upload stalled: nothing arrived for a minute")
			return apiFailure(408, "transfer_stalled", "The upload stalled: nothing arrived for a minute")
		}
		if n > 0 {
			writing, cancel := context.WithTimeout(ctx, transferIdle)
			failed := writer.Write(writing, buffer[:n])
			stalled := errors.Is(writing.Err(), context.DeadlineExceeded)
			cancel()
			if ctx.Err() != nil {
				return changed()
			}
			if stalled {
				writer.Fail("the Host took nothing for too long")
				return apiFailure(504, "host_operation_failed", "The Host took nothing for too long")
			}
			if failed != nil {
				break
			}
		}
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			writer.Fail("the upload was cut short")
			return apiFailure(400, "invalid_body", "The upload was cut short: "+err.Error())
		}
	}
	writer.End()
	if err := written(ctx); err != nil {
		if ctx.Err() != nil {
			return changed()
		}
		return err
	}
	return nil
}
func copyDownload(ctx context.Context, to io.Writer, from host.ByteStream) error {
	buffer := make([]byte, 64*1024)
	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		n, err := from.Read(ctx, buffer)
		if n > 0 {
			if _, failed := to.Write(buffer[:n]); failed != nil {
				return failed
			}
		}
		if errors.Is(err, io.EOF) {
			return nil
		}
		if err != nil {
			return err
		}
	}
}

type downloadLease interface {
	Context() context.Context
	Release()
}

func streamDownload(w http.ResponseWriter, r *http.Request, download *hostaccess.DownloadStream) error {
	peer := r.Context().Value(connectionKey{}).(*activity)
	return serveDownload(r.Context(), w, peer, download.Body, download.Lease, download.Part.Status())
}

// serveDownload holds admission until the bytes finish, fail, or the visitor
// leaves. Both the request's lifetime and the lease can end a blocked Host read.
func serveDownload(ctx context.Context, w http.ResponseWriter, peer *activity, body host.ByteStream, lease downloadLease, status int) error {
	defer lease.Release()
	// The copy reports IO failures; closing the source only releases resources.
	defer func() { _ = body.Close(context.WithoutCancel(ctx)) }()
	copying, cancel := context.WithCancel(lease.Context())
	defer cancel()
	ended := make(chan struct{})
	stopRequest := context.AfterFunc(ctx, func() {
		defer close(ended)
		cancel()
	})
	defer func() {
		if !stopRequest() {
			<-ended
		}
	}()
	stop := peer.watch(copying, transferIdle)
	defer stop()
	w.WriteHeader(status)
	if err := copyDownload(copying, w, body); err != nil {
		// Closing prevents a final chunk from turning a failed read into success.
		_ = peer.Close()
		return err
	}
	return nil
}
