package edge

import (
	"context"
	"errors"
	"net/http"

	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// onHost admits each conversation file operation through its owning shard.
func onHost[T any](
	e *Edge,
	r *http.Request,
	operation func(context.Context, *hostaccess.ConversationHost) (T, error),
) (T, error) {
	var zero T
	id, err := webapi.ParseConversationID(r.PathValue("id"))
	if err != nil {
		return zero, apiFailure(404, "conversation_not_found", "No such conversation")
	}
	var device *webapi.DeviceID
	if text := r.PathValue("device"); text != "" {
		parsed, err := webapi.ParseDeviceID(text)
		if err != nil {
			return zero, apiFailure(404, "host_not_attached", "No such attached host")
		}
		device = &parsed
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return zero, err
	}
	return hostaccess.WithHost(r.Context(), shard.HostShard(), id, device, operation)
}

func (e *Edge) directory(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeDirectoryQuery)
	if err != nil {
		return err
	}
	result, err := onHost(e, r, func(ctx context.Context, h *hostaccess.ConversationHost) (webapi.Directory, error) {
		path := h.Root
		if query.Path != nil {
			path = string(*query.Path)
		}
		entries, err := runners.BrowseDirectory(ctx, h.Host.FS(), path)
		return webapi.Directory{Path: path, Home: h.Home, Entries: entries}, err
	})
	if err != nil {
		return err
	}
	writeJSON(w, 200, result)
	return nil
}

func (e *Edge) mkdir(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeCreateDirectory)
	if err != nil {
		return err
	}
	result, err := onHost(
		e,
		r,
		func(ctx context.Context, h *hostaccess.ConversationHost) (webapi.CreatedDirectory, error) {
			return webapi.CreatedDirectory(
					request,
				), h.Host.FS().
					Mkdir(ctx, request.Path, host.MkdirOptions{Recursive: true})
		},
	)
	if err != nil {
		return err
	}
	writeJSON(w, 201, result)
	return nil
}

func (e *Edge) removeFile(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeRemoveQuery)
	if err != nil {
		return err
	}
	_, err = onHost(e, r, func(ctx context.Context, h *hostaccess.ConversationHost) (struct{}, error) {
		kept := []string{h.Root}
		if h.Home != nil {
			kept = append(kept, *h.Home)
		}
		if protectedPath(string(query.Path), kept...) {
			return struct{}{}, apiFailure(
				409,
				"protected_path",
				"The root, the home directory and the execution directory stay, with every directory holding them",
			)
		}
		return struct{}{}, h.Host.FS().Rm(ctx, string(query.Path), host.RmOptions{Recursive: true, Force: true})
	})
	if err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}

func (e *Edge) fileText(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeFileQuery)
	if err != nil {
		return err
	}
	result, err := onHost(e, r, func(ctx context.Context, h *hostaccess.ConversationHost) (webapi.FileText, error) {
		text, err := runners.ReadTextFile(ctx, h.Host.FS(), string(query.Path))
		return webapi.FileText{Path: string(query.Path), Text: text}, err
	})
	if err != nil {
		return err
	}
	writeJSON(w, 200, result)
	return nil
}

func (e *Edge) changes(w http.ResponseWriter, r *http.Request) error {
	result, err := onHost(
		e,
		r,
		func(ctx context.Context, h *hostaccess.ConversationHost) (webapi.WorkingTreeChanges, error) {
			changes, err := h.Host.GitChanges(ctx, h.Root)
			return webapi.WorkingTreeChanges{Root: h.Root, GitChanges: changes}, err
		},
	)
	if err != nil {
		return workingTreeError(err)
	}
	writeJSON(w, 200, result)
	return nil
}

func (e *Edge) changedFile(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeTreeFileQuery)
	if err != nil {
		return err
	}
	sides, err := onHost(e, r, func(ctx context.Context, h *hostaccess.ConversationHost) (webapi.ChangeSides, error) {
		var sides webapi.ChangeSides
		bytes, err := h.Host.GitShow(ctx, h.Root, string(query.Path))
		var missing *host.Error
		if err != nil && (!errors.As(err, &missing) || missing.Code != "ENOENT") {
			return sides, workingTreeError(err)
		}
		sides.Original, err = runners.TextOf(bytes)
		if err != nil {
			return sides, err
		}
		sides.Modified, err = runners.ReadTextFile(ctx, h.Host.FS(), h.Root+"/"+string(query.Path))
		if errors.As(err, &missing) && missing.Code == "ENOENT" {
			err = nil
		}
		return sides, err
	})
	if err != nil {
		return err
	}
	writeJSON(w, 200, sides)
	return nil
}

func (e *Edge) committedFile(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeCommittedFileQuery, "download")
	if err != nil {
		return err
	}
	bytes, err := onHost(e, r, func(ctx context.Context, h *hostaccess.ConversationHost) ([]byte, error) {
		bytes, err := h.Host.GitShow(ctx, h.Root, string(query.Path))
		return bytes, workingTreeError(err)
	})
	if err != nil {
		return err
	}
	part := hostaccess.RangeOf(r.Header.Get("Range"), uint64(len(bytes)))
	addHeaders(w.Header(), rawFileHeaders())
	addHeaders(
		w.Header(),
		contentHeaders(core.PreviewMediaType(string(query.Path)), bool(query.Download), fileName(string(query.Path))),
	)
	addHeaders(w.Header(), part.Headers())
	w.WriteHeader(part.Status())
	if r.Method != "HEAD" {
		_, err = w.Write(part.PartOf(bytes))
	}
	return err
}
