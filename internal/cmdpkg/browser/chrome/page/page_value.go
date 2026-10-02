package page

import (
	"bytes"
	"encoding/json"
	"fmt"
	"reflect"

	jsonv2 "github.com/go-json-experiment/json"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// decodePageValue validates page algorithm results before they become Go values.
// json/v2 supplies duplicate, UTF-8 and field validation; Rust's non-null scalar
// and exact-length array rules are not available as json/v2 decode options.
func decodePageValue[T any](raw []byte) (T, error) {
	var value T
	kind := reflect.TypeFor[T]().Kind()
	if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) && kind != reflect.Pointer && reflect.TypeFor[T]() != reflect.TypeFor[json.RawMessage]() {
		return value, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "unexpected null browser result"}
	}
	if kind == reflect.Array {
		var items []json.RawMessage
		if err := jsonv2.Unmarshal(raw, &items); err != nil {
			return value, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
		}
		if len(items) != reflect.TypeFor[T]().Len() {
			return value, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: fmt.Sprintf("invalid length %d, expected an array of length %d", len(items), reflect.TypeFor[T]().Len())}
		}
		for _, item := range items {
			if bytes.Equal(bytes.TrimSpace(item), []byte("null")) {
				return value, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "unexpected null browser coordinate"}
			}
		}
	}
	if err := jsonv2.Unmarshal(raw, &value, jsonv2.RejectUnknownMembers(true)); err != nil {
		return value, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	return value, nil
}
