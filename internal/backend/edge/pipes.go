package edge

import (
	"context"
	"errors"
	"io"
	"net/http"
	"strings"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/webapi"
)

func plain(w http.ResponseWriter, status int, text string) {
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.WriteHeader(status)
	_, _ = io.WriteString(w, text)
}

func (e *Edge) pipe(w http.ResponseWriter, r *http.Request) error {
	scheme, token, ok := strings.Cut(r.Header.Get("Authorization"), " ")
	if !ok || !strings.EqualFold(scheme, "Bearer") || token == "" {
		plain(w, 401, "device token required")
		return nil
	}
	device, found, err := e.state.Services.Control.DeviceByToken(r.Context(), database.HashToken(token))
	if err != nil {
		w.WriteHeader(500)
		return nil
	}
	if !found {
		plain(w, 401, "device token required")
		return nil
	}
	shard, err := e.state.Shards.OfWhileClosing(r.Context(), device.User)
	if err != nil {
		w.WriteHeader(503)
		return nil
	}
	return servePipe(w, r, shard.Pipes(), device.ID)
}

func servePipe(w http.ResponseWriter, r *http.Request, pipes *remotehost.Pipes, deviceID webapi.DeviceID) error {
	refused := func(err error) {
		status := 409
		var refusal remotehost.PipeRefusal
		if errors.As(err, &refusal) && refusal == remotehost.ErrPipeNotFound {
			status = 404
		}
		plain(w, status, err.Error())
	}
	if r.Method == "PUT" {
		source, err := pipes.ClaimSource(r.PathValue("id"), string(deviceID))
		if err != nil {
			refused(err)
			return nil
		}
		err = source.Pump(r.Context(), &bodyStream{body: r.Body, control: http.NewResponseController(w)})
		if err != nil {
			plain(w, 409, err.Error())
		} else {
			plain(w, 200, "drained")
		}
		return nil
	}
	sink, err := pipes.ClaimSink(r.PathValue("id"), string(deviceID))
	if err != nil {
		refused(err)
		return nil
	}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = sink.Close(context.WithoutCancel(r.Context())) }()
	if err := sink.SourceArrived(r.Context()); err != nil {
		plain(w, 409, err.Error())
		return nil
	}
	w.Header().Set("Content-Type", "application/octet-stream")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(200)
	if err := copyDownload(r.Context(), w, sink); err != nil {
		_ = r.Context().Value(connectionKey{}).(*activity).Close()
	}
	return nil
}
