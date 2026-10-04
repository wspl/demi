package webapiproto

import (
	"fmt"
	"net/url"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// DecodeDeviceLogValues reads the device log query, including its default limit.
// Unknown keys are ignored.
func DecodeDeviceLogValues(values url.Values) (DeviceLogQuery, error) {
	fields := []contract.Field{{Name: "limit", Value: DefaultLogLimit}}
	for _, key := range []string{"since", "limit", "source"} {
		texts, ok := values[key]
		if !ok {
			continue
		}
		if len(texts) != 1 {
			return DeviceLogQuery{}, fmt.Errorf("%s: expected one value", key)
		}
		var value any = texts[0]
		if key != "source" {
			number, err := strconv.ParseUint(strings.TrimPrefix(texts[0], "+"), 10, 64)
			if err != nil {
				return DeviceLogQuery{}, fmt.Errorf("%s: %w", key, err)
			}
			value = number
		}
		if key == "limit" {
			fields[0].Value = value
		} else {
			fields = append(fields, contract.Field{Name: key, Value: value})
		}
	}
	data, err := contract.EncodeObject(fields)
	if err != nil {
		return DeviceLogQuery{}, err
	}
	return DecodeDeviceLogQuery(data)
}

// UnmarshalText implements the exact boolean spelling of query parameters.
func (b *StrictBool) UnmarshalText(text []byte) error {
	next, err := ParseStrictBool(string(text))
	if err != nil {
		return err
	}
	*b = next
	return nil
}
