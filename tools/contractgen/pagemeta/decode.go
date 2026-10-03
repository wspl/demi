package pagemeta

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"reflect"
)

// Decode rejects incomplete metadata, unknown fields and trailing data.
func Decode(data []byte) ([]Page, error) {
	var pages []Page
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&pages); err != nil {
		return nil, err
	}
	if err := decoder.Decode(new(any)); !errors.Is(err, io.EOF) {
		if err != nil {
			return nil, err
		}
		return nil, fmt.Errorf("trailing page metadata")
	}
	if err := required(data, reflect.TypeFor[[]Page]()); err != nil {
		return nil, err
	}
	for _, page := range pages {
		for _, schema := range page.Schemas {
			if schema.Direction != "receive" && schema.Direction != "send" {
				return nil, fmt.Errorf("plugin %s: invalid direction %q", page.ID, schema.Direction)
			}
		}
	}
	return pages, nil
}

// required checks presence in this tool interface without a second shape declaration.
func required(data []byte, typ reflect.Type) error {
	if typ == reflect.TypeFor[json.RawMessage]() {
		return nil
	}
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		return fmt.Errorf("null %s in page metadata", typ)
	}
	switch typ.Kind() {
	case reflect.Slice:
		var items []json.RawMessage
		if err := json.Unmarshal(data, &items); err != nil {
			return err
		}
		for _, item := range items {
			if err := required(item, typ.Elem()); err != nil {
				return err
			}
		}
	case reflect.Struct:
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(data, &fields); err != nil {
			return err
		}
		for i := 0; i < typ.NumField(); i++ {
			field := typ.Field(i)
			name := field.Tag.Get("json")
			value, ok := fields[name]
			if !ok {
				return fmt.Errorf("missing page metadata field %s", name)
			}
			if err := required(value, field.Type); err != nil {
				return fmt.Errorf("%s: %w", name, err)
			}
			delete(fields, name)
		}
		for name := range fields {
			return fmt.Errorf("unknown page metadata field %s", name)
		}
	}
	return nil
}
