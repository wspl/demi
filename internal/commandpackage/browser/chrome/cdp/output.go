package cdp

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/contract"
)

// Value converts a typed result to the JSON value Render bounds, using the
// shared contract encoder, which applies no HTML or JavaScript escaping.
func Value(result any) (json.RawMessage, error) {
	data, err := contract.EncodeJSON(result)
	if err != nil {
		return nil, &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
	}
	return json.RawMessage(data), nil
}

// Resolve returns the absolute command output path against cwd. Empty paths
// and paths containing NUL are invalid input.
func Resolve(cwd, path string) (string, error) {
	resolved, err := commandsdk.Resolve(cwd, path)
	if err != nil {
		return "", &BrowserError{Kind: KindConfiguration, Message: err.Error(), Cause: err}
	}
	return resolved, nil
}

// Preflight rejects an existing browser output before delivering page input.
func Preflight(ctx context.Context, cwd, output string, overwrite bool) (string, error) {
	if err := ctx.Err(); err != nil {
		return "", err
	}
	path, err := Resolve(cwd, output)
	if err != nil {
		return "", err
	}
	_, err = os.Lstat(path)
	if err == nil && !overwrite {
		return "", &BrowserError{Kind: KindOutputExists, Message: path}
	}
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return "", &BrowserError{Kind: KindIO, Cause: err}
	}
	return path, nil
}

// SaveWithOverwrite publishes asset bytes with the explicit overwrite policy.
// The context supplies cancellation and the command's shared deadline.
func SaveWithOverwrite(ctx context.Context, cwd, output string, data []byte, overwrite bool) (string, error) {
	path, err := Preflight(ctx, cwd, output, overwrite)
	if err != nil {
		return "", err
	}
	mode := artifacts.CreateNew
	if overwrite {
		mode = artifacts.Replace
	}
	err = artifacts.PublishBytes(ctx, path, data, artifacts.Publication{Mode: mode})
	return publicationResult(path, err)
}

// PublishFile atomically publishes a completed download without buffering it.
func PublishFile(ctx context.Context, cwd, output, source string, overwrite bool) (path string, err error) {
	path, err = Preflight(ctx, cwd, output, overwrite)
	if err != nil {
		return "", err
	}
	file, err := os.Open(source)
	if err != nil {
		return "", &BrowserError{Kind: KindIO, Cause: err}
	}
	defer func() {
		if closeErr := file.Close(); closeErr != nil {
			err = AfterCleanup(err, &BrowserError{Kind: KindIO, Cause: closeErr})
		}
	}()
	mode := artifacts.CreateNew
	if overwrite {
		mode = artifacts.Replace
	}
	err = artifacts.Publish(ctx, path, file, artifacts.Publication{Mode: mode})
	return publicationResult(path, err)
}

// publicationResult translates atomic browser output failures without late writes.
func publicationResult(path string, err error) (string, error) {
	if err == nil {
		return path, nil
	}
	switch {
	case errors.Is(err, os.ErrExist):
		return "", &BrowserError{Kind: KindOutputExists, Message: path, Cause: err}
	case errors.Is(err, context.DeadlineExceeded):
		return "", &BrowserError{Kind: KindTimeout, Cause: err}
	case errors.Is(err, context.Canceled):
		return "", &BrowserError{Kind: KindCancelled, Cause: err}
	default:
		return "", &BrowserError{Kind: KindIO, Cause: err}
	}
}

// Render bounds a result before success bytes escape, shrinking supported
// content and collections while preserving the first omitted stream position.
func Render(operation browserproto.Operation, value json.RawMessage, asJSON bool) ([]byte, error) {
	var object map[string]json.RawMessage
	if err := json.Unmarshal(value, &object); err != nil {
		return nil, &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
	}
	for {
		encoded, err := Value(object)
		if err != nil {
			return nil, err
		}
		value = encoded
		if len(value) <= browserproto.InlineBytes {
			break
		}
		var content string
		if raw, ok := object["content"]; ok && json.Unmarshal(raw, &content) == nil && content != "" {
			end := len(content) / 2
			for end > 0 && !utf8.RuneStart(content[end]) {
				end--
			}
			shortened, err := Value(content[:end])
			if err != nil {
				return nil, err
			}
			object["content"] = shortened
			object["truncated"] = json.RawMessage(`true`)
			continue
		}
		if err := shortenCollection(object); err != nil {
			return nil, err
		}
	}
	text := string(value)
	if !asJSON {
		var err error
		text, err = renderText(operation, value)
		if err != nil {
			return nil, err
		}
	}
	if !strings.HasSuffix(text, "\n") {
		text += "\n"
	}
	if len(text) > browserproto.InlineBytes {
		return nil, &BrowserError{Kind: KindResultTooLarge}
	}
	return []byte(text), nil
}

// RenderError renders failures with the progress and details of their JSON form.
func RenderError(code browserproto.BrowserErrorCode, message string, details browserproto.ErrorDetails) string {
	action := browserproto.ActionProgress("not_started")
	if details.Action != nil {
		action = *details.Action
	}
	text := fmt.Sprintf("Error: %s\n%s\nAction: %s.\n", code, plain(message), action)
	raw, err := Value(details)
	if err != nil {
		return text
	} // Details that cannot be serialized are omitted.
	var fields map[string]any
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	if decoder.Decode(&fields) != nil {
		return text
	}
	keys := make([]string, 0, len(fields))
	for key := range fields {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		if key == "action" {
			continue
		}
		title := strings.ToUpper(key[:1]) + key[1:]
		if key == "url" {
			title = "Current URL"
		}
		text += title + ": " + pageValue(fields[key]) + "\n"
	}
	return text
}

func shortenCursor(object map[string]json.RawMessage, items []json.RawMessage, omitted json.RawMessage) error {
	var cursor string
	raw, ok := object["cursor"]
	if !ok || json.Unmarshal(raw, &cursor) != nil {
		return nil
	}
	if len(items) == 0 {
		return &BrowserError{Kind: KindResultTooLarge}
	}
	index := strings.LastIndexByte(cursor, ':')
	if index < 0 {
		return &BrowserError{Kind: KindInvalidResult, Message: "stream cursor has no position"}
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(omitted, &fields); err != nil {
		return &BrowserError{
			Kind:    KindInvalidResult,
			Message: "stream entry has no sequence",
			Cause:   err,
		}
	}
	var sequence uint64
	if err := json.Unmarshal(fields["sequence"], &sequence); err != nil {
		return &BrowserError{
			Kind:    KindInvalidResult,
			Message: "stream entry has no sequence",
			Cause:   err,
		}
	}
	encoded, err := Value(fmt.Sprintf("%s:%d", cursor[:index], sequence))
	if err != nil {
		return err
	}
	object["cursor"] = encoded
	object["hasMore"] = json.RawMessage(`true`)

	return nil
}

func shortenCollection(object map[string]json.RawMessage) error {
	shortened := false
	for _, key := range []string{"tree", "matches", "entries", "events", "tabs", "values"} {
		var items []json.RawMessage
		raw, ok := object[key]
		if !ok || len(raw) == 0 || raw[0] != '[' || json.Unmarshal(raw, &items) != nil {
			continue
		}
		if len(items) == 0 {
			break
		}
		omitted := items[len(items)-1]
		items = items[:len(items)-1]
		encoded, err := Value(items)
		if err != nil {
			return err
		}
		object[key] = encoded
		if err := shortenCursor(object, items, omitted); err != nil {
			return err
		}
		object["truncated"] = json.RawMessage(`true`)
		shortened = true
		break
	}
	if !shortened {
		return &BrowserError{Kind: KindResultTooLarge}
	}
	return nil
}
