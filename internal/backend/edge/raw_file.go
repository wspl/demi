package edge

import (
	"errors"
	"net/http"

	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

func fileHeaders(path string, download bool, stat host.FileStat, part hostaccess.RangeAnswer) (http.Header, error) {
	result := rawFileHeaders()
	mediaType, _ := core.PreviewMediaType(path)
	addHeaders(result, contentHeaders(mediaType, download, fileName(path)))
	result.Set("ETag", hostaccess.FileVersion(stat))
	modified, err := lastModified(stat)
	if err != nil {
		return nil, err
	}
	result.Set("Last-Modified", modified)
	addHeaders(result, part.Headers())
	return result, nil
}

func (e *Edge) rawFile(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeRawFileQuery, "download")
	if err != nil {
		return err
	}
	id, err := webapi.ParseConversationID(r.PathValue("id"))
	if err != nil {
		return apiFailure(404, "conversation_not_found", "No such conversation")
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	var version *string
	if query.Version != nil {
		v := string(*query.Version)
		version = &v
	}
	answer, err := hostaccess.OpenDownload(
		r.Context(),
		shard.HostShard(),
		id,
		hostaccess.DownloadRequest{
			Path:        string(query.Path),
			Version:     version,
			Head:        r.Method == "HEAD",
			Range:       r.Header.Get("Range"),
			IfNoneMatch: r.Header.Get("If-None-Match"),
		},
	)
	switch {
	case errors.Is(err, hostaccess.ErrNotAFile):
		return apiFailure(404, "not_found", "Not a regular file")
	case errors.Is(err, hostaccess.ErrFileChanged):
		return apiFailure(412, "file_changed", "The file is no longer the version asked for")
	case err != nil:
		return err
	}
	return writeDownload(w, r, query, answer)
}

func (e *Edge) uploadFile(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeFileUploadQuery, "replace")
	if err != nil {
		return err
	}
	id, err := webapi.ParseConversationID(r.PathValue("id"))
	if err != nil {
		return apiFailure(404, "conversation_not_found", "No such conversation")
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	answer, err := hostaccess.UploadFile(r.Context(), shard.HostShard(), id, string(query.Path), bool(query.Replace))
	switch {
	case errors.Is(err, hostaccess.ErrIsDirectory):
		return apiFailure(409, "is_directory", "A directory is at this path")
	case errors.Is(err, hostaccess.ErrFileExists):
		return apiFailure(409, "file_exists", "A file is already at this path")
	case err != nil:
		return err
	}
	defer answer.Lease.Release()
	err = copyUpload(
		answer.Lease.Context(),
		&bodyStream{body: r.Body, control: http.NewResponseController(w)},
		answer.Writer,
		answer.Written,
	)
	if failure, ok := err.(*apiError); ok && failure.status == 504 {
		_ = r.Context().Value(connectionKey{}).(*activity).Close()
	}
	if err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}

func writeDownload(
	w http.ResponseWriter,
	r *http.Request,
	query webapi.RawFileQuery,
	answer hostaccess.Download,
) error {
	switch answer := answer.(type) {
	case *hostaccess.DownloadNotModified:
		addHeaders(w.Header(), rawFileHeaders())
		w.Header().Set("ETag", answer.Version)
		w.WriteHeader(304)
	case *hostaccess.DownloadHead:
		headers, err := fileHeaders(string(query.Path), bool(query.Download), answer.Stat, answer.Part)
		if err != nil {
			return err
		}
		addHeaders(w.Header(), headers)
		w.WriteHeader(answer.Part.Status())
	case *hostaccess.DownloadStream:
		headers, err := fileHeaders(string(query.Path), bool(query.Download), answer.Stat, answer.Part)
		if err != nil {
			answer.Lease.Release()
			_ = answer.Body.Close(r.Context())
			return err
		}
		headers.Del("Content-Length")
		addHeaders(w.Header(), headers)
		return streamDownload(w, r, answer)
	}
	return nil
}
