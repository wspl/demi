package contract

import (
	"bytes"
	"encoding/json"
	"errors"

	"github.com/vmihailenco/msgpack/v5"
	"github.com/vmihailenco/msgpack/v5/msgpcode"
)

// ObjectFields reads JSON properties in wire order after checking the boundary.
func ObjectFields(data []byte) ([]Field, error) {
	if err := CheckJSON(data); err != nil {
		return nil, err
	}
	d := json.NewDecoder(bytes.NewReader(data))
	token, err := d.Token()
	if err != nil {
		return nil, err
	}
	if token != json.Delim('{') {
		return nil, errors.New("expected object")
	}
	fields := []Field{}
	for d.More() {
		token, err := d.Token()
		if err != nil {
			return nil, err
		}
		key, ok := token.(string)
		if !ok {
			return nil, errors.New("expected string key")
		}
		var raw json.RawMessage
		if err := d.Decode(&raw); err != nil {
			return nil, At(key, err)
		}
		fields = append(fields, Field{Name: key, Value: raw})
	}
	return fields, nil
}

// MsgpackFields reads MessagePack properties in wire order after checking them.
func MsgpackFields(data []byte) ([]Field, error) {
	if err := CheckMsgpack(data); err != nil {
		return nil, err
	}
	d := msgpack.NewDecoder(bytes.NewReader(data))
	code, err := d.PeekCode()
	if err != nil {
		return nil, err
	}
	if !msgpcode.IsFixedMap(code) && code != msgpcode.Map16 && code != msgpcode.Map32 {
		return nil, errors.New("expected object")
	}
	n, err := d.DecodeMapLen()
	if err != nil {
		return nil, err
	}
	fields := make([]Field, 0, n)
	for range n {
		key, err := d.DecodeString()
		if err != nil {
			return nil, err
		}
		raw, err := d.DecodeRaw()
		if err != nil {
			return nil, At(key, err)
		}
		fields = append(fields, Field{Name: key, Value: raw})
	}
	return fields, nil
}

// AdjacentFields extracts a flattened union without discarding its wire order.
// The containing generated decoder checks sibling and unknown properties.
func AdjacentFields(fields []Field, tag, content string) ([]Field, error) {
	out := []Field{}
	seen := false
	for _, field := range fields {
		switch field.Name {
		case tag:
			seen = true
			out = append(out, field)
		case content:
			if !seen {
				return nil, At(content, errors.New("tag must precede content"))
			}
			out = append(out, field)
		}
	}
	if len(out) != 2 {
		return nil, errors.New("missing adjacent tag or content")
	}
	return out, nil
}
