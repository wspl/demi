package edge

import (
	"encoding/json/v2"
	"errors"
	"io"
	"log/slog"
	"net/http"
	"strconv"

	"github.com/wspl/demi/go/webapi"
)

// jsonBodyLimit is the most of a JSON body the backend reads (web-api.md §
// Request bodies).
const jsonBodyLimit = 1024 * 1024

func tooLarge() *apiError {
	return newError(http.StatusRequestEntityTooLarge, webapi.ErrorCodeTooLarge, "The request body is over its "+strconv.Itoa(jsonBodyLimit)+"-byte limit")
}

// readBody is the request's body, at most jsonBodyLimit bytes: a larger
// declared length is refused before any byte is read, and a body that turns
// out larger when read.
func readBody(w http.ResponseWriter, r *http.Request) ([]byte, error) {
	if r.ContentLength > jsonBodyLimit {
		return nil, tooLarge()
	}
	data, err := io.ReadAll(http.MaxBytesReader(w, r.Body, jsonBodyLimit))
	var over *http.MaxBytesError
	if errors.As(err, &over) {
		return nil, tooLarge()
	}
	if err != nil {
		return nil, invalidBody(err.Error())
	}
	return data, nil
}

// jsonBody is the request's JSON body decoded as T and checked by T's
// rules, whatever its content type; a body that is empty, malformed or does
// not match T is refused as invalid_body naming the field and the reason.
func jsonBody[T any](w http.ResponseWriter, r *http.Request) (T, error) {
	var zero T
	data, err := readBody(w, r)
	if err != nil {
		return zero, err
	}
	value, err := webapi.Decode[T](data)
	if err != nil {
		return zero, invalidBody(err.Error())
	}
	return value, nil
}

// optionalJSONBody is jsonBody, but an empty body reads as T's zero value.
func optionalJSONBody[T any](w http.ResponseWriter, r *http.Request) (T, error) {
	var zero T
	data, err := readBody(w, r)
	if err != nil || len(data) == 0 {
		return zero, err
	}
	value, err := webapi.Decode[T](data)
	if err != nil {
		return zero, invalidBody(err.Error())
	}
	return value, nil
}

// writeJSON answers value as JSON with status.
func writeJSON(w http.ResponseWriter, status int, value any) {
	data, err := json.Marshal(value)
	if err != nil {
		slog.Error("an answer could not be encoded", "error", err)
		status = http.StatusInternalServerError
		data, _ = json.Marshal(webapi.ErrorBody{Code: webapi.ErrorCodeInternalError, Message: err.Error()})
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	// A client that went away reads no answer.
	_, _ = w.Write(data)
}
