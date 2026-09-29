package storage

import (
	"bytes"
	jsonv1 "encoding/json"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"reflect"
	"strconv"
	"strings"
)

// commandValuesEqual compares the Rust command map's JSON numbers without
// losing integer precision or conflating an integer with a floating-point value.
func commandValuesEqual(old, new map[string]jsontext.Value) (bool, error) {
	decode := func(value map[string]jsontext.Value) (any, error) {
		data, err := json.Marshal(value)
		if err != nil {
			return nil, err
		}
		reader := jsonv1.NewDecoder(bytes.NewReader(data))
		reader.UseNumber()
		var decoded any
		if err = reader.Decode(&decoded); err != nil {
			return nil, err
		}
		return commandNumbers(decoded)
	}
	a, err := decode(old)
	if err != nil {
		return false, err
	}
	b, err := decode(new)
	if err != nil {
		return false, err
	}
	return reflect.DeepEqual(a, b), nil
}
func commandNumbers(value any) (any, error) {
	switch v := value.(type) {
	case jsonv1.Number:
		text := string(v)
		if strings.ContainsAny(text, ".eE") || text == "-0" {
			return strconv.ParseFloat(text, 64)
		}
		if strings.HasPrefix(text, "-") {
			return strconv.ParseInt(text, 10, 64)
		}
		n, err := strconv.ParseUint(text, 10, 64)
		if err == nil {
			return n, nil
		}
		return strconv.ParseFloat(text, 64)
	case []any:
		for i, item := range v {
			converted, err := commandNumbers(item)
			if err != nil {
				return nil, err
			}
			v[i] = converted
		}
	case map[string]any:
		for key, item := range v {
			converted, err := commandNumbers(item)
			if err != nil {
				return nil, err
			}
			v[key] = converted
		}
	}
	return value, nil
}
