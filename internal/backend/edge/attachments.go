package edge

import (
	"fmt"
	"io"
	"mime"
	"net/http"
	"strings"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) attachment(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeUploadQuery)
	if err != nil {
		return err
	}
	media, params, err := mime.ParseMediaType(r.Header.Get("Content-Type"))
	if err != nil || !strings.Contains(media, "/") || strings.HasPrefix(media, "multipart/") {
		return apiFailure(400, "invalid_body", "Send the file's bytes with its media type as Content-Type")
	}
	sent := mime.FormatMediaType(media, params)
	tooLarge := apiFailure(413, "too_large", fmt.Sprintf("An upload is at most %d bytes", webapi.AttachmentMaxBytes))
	if r.ContentLength > webapi.AttachmentMaxBytes {
		return tooLarge
	}
	bytes, err := io.ReadAll(io.LimitReader(r.Body, webapi.AttachmentMaxBytes+1))
	if err != nil {
		return invalidBody(err)
	}
	if len(bytes) > webapi.AttachmentMaxBytes {
		return tooLarge
	}
	if len(bytes) == 0 {
		return apiFailure(400, "invalid_body", "An upload holds at least one byte")
	}
	media = store.UploadMediaType(sent, bytes)
	var opening *string
	if store.IsText(query.Name, media) {
		snippet := store.Snippet(bytes)
		opening = &snippet
	}
	ref, err := e.state.Services.Blobs.ForUser(caller(r).ID).Put(r.Context(), bytes)
	if err != nil {
		return err
	}
	record, err := e.state.Services.Control.CreateAttachment(
		r.Context(),
		caller(r).ID,
		media,
		uint64(len(bytes)),
		ref,
		opening,
	)
	if err != nil {
		return err
	}
	writeJSON(
		w,
		201,
		webapi.AttachmentAnswer{
			Attachment: webapi.AttachmentDTO{
				ID:        record.ID,
				MediaType: record.MediaType,
				SizeBytes: record.SizeBytes,
				Sha256:    record.SHA256,
				CreatedAt: record.CreatedAt,
				Snippet:   record.Snippet,
			},
		},
	)
	return nil
}

func (e *Edge) blob(w http.ResponseWriter, r *http.Request) error {
	missing := apiFailure(404, "not_found", "No such blob")
	ref, err := core.ParseBlobRef(r.PathValue("sha256"))
	if err != nil {
		return missing
	}
	bytes, found, err := e.state.Services.Blobs.ForUser(caller(r).ID).Read(r.Context(), ref)
	if err != nil {
		return err
	}
	if !found {
		return missing
	}
	var media *string
	if values := r.URL.Query()["type"]; len(values) > 0 {
		media = &values[0]
	}
	part := hostaccess.RangeOf(r.Header.Get("Range"), uint64(len(bytes)))
	addHeaders(w.Header(), contentHeaders(media, false, ""))
	w.Header().Set("Cache-Control", "private, max-age=31536000, immutable")
	w.Header().Set("Vary", "Cookie")
	addHeaders(w.Header(), part.Headers())
	w.WriteHeader(part.Status())
	if r.Method != "HEAD" {
		_, err = w.Write(part.PartOf(bytes))
	}
	return err
}
