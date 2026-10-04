package database

import (
	"bytes"
	"encoding/json"
	"reflect"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// equalCommandValues compares immutable snapshots as decoded JSON values, where a
// number is a uint64, a negative int64 or a float64, so 1 and 1.0 differ.
// Canonical JSON digests use IEEE doubles and would lose distinct large integer values here.
func equalCommandValues(left, right commandValues) (bool, error) {
	if len(left) != len(right) {
		return false, nil
	}
	for key, a := range left {
		b, ok := right[key]
		if !ok {
			return false, nil
		}
		av, err := comparableJSONValue(a)
		if err != nil {
			return false, err
		}
		bv, err := comparableJSONValue(b)
		if err != nil {
			return false, err
		}
		if !reflect.DeepEqual(av, bv) {
			return false, nil
		}
	}
	return true, nil
}

// comparableJSONValue interprets validated stored JSON for comparison, decoding each
// number as a uint64, a negative int64 or, otherwise, a float64. It writes no JSON.
func comparableJSONValue(raw json.RawMessage) (any, error) {
	if _, err := contract.JSON(raw); err != nil {
		return nil, err
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	var value any
	if err := decoder.Decode(&value); err != nil {
		return nil, err
	}
	return comparableJSONNumbers(value)
}

func comparableJSONNumbers(value any) (any, error) {
	switch v := value.(type) {
	case json.Number:
		text := string(v)
		if !strings.ContainsAny(text, ".eE") {
			if n, err := strconv.ParseUint(text, 10, 64); err == nil {
				return n, nil
			}
			if n, err := strconv.ParseInt(text, 10, 64); err == nil && n < 0 {
				return n, nil
			}
		}
		return strconv.ParseFloat(text, 64)
	case []any:
		for i, item := range v {
			normalized, err := comparableJSONNumbers(item)
			if err != nil {
				return nil, err
			}
			v[i] = normalized
		}
	case map[string]any:
		for key, item := range v {
			normalized, err := comparableJSONNumbers(item)
			if err != nil {
				return nil, err
			}
			v[key] = normalized
		}
	}
	return value, nil
}
