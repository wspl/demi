package edge

import (
	"fmt"
	"io"
	"net/http"
)

const jsonBodyLimit = 1024 * 1024

// readJSONBody bounds the whole-body allocation before the generated decoder.
func readJSONBody(r *http.Request) ([]byte, error) {
	tooLarge := func() error {
		return apiFailure(413, "too_large", fmt.Sprintf("The request body is over its %d-byte limit", jsonBodyLimit))
	}
	if r.ContentLength > jsonBodyLimit {
		return nil, tooLarge()
	}
	bytes, err := io.ReadAll(io.LimitReader(r.Body, jsonBodyLimit+1))
	if err != nil {
		return nil, invalidBody(err)
	}
	if len(bytes) > jsonBodyLimit {
		return nil, tooLarge()
	}
	return bytes, nil
}

func decodeBody[T any](r *http.Request, decode func([]byte) (T, error)) (T, error) {
	var zero T
	data, err := readJSONBody(r)
	if err != nil {
		return zero, err
	}
	value, err := decode(data)
	if err != nil {
		return zero, invalidBody(err)
	}
	return value, nil
}
