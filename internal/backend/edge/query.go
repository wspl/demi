package edge

import (
	"fmt"
	"net/http"
	"net/url"
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

// decodeQuery converts a route's query into its generated contract decoder.
// Only the named boolean keys change JSON kind; everything else stays text.
func decodeQuery[T any](r *http.Request, decode func([]byte) (T, error), booleans ...string) (T, error) {
	var zero T
	values, err := url.ParseQuery(r.URL.RawQuery)
	if err != nil {
		return zero, apiFailure(400, "invalid_query", err.Error())
	}
	fields := make([]contract.Field, 0, len(values))
	for key, texts := range values {
		if len(texts) != 1 {
			return zero, apiFailure(400, "invalid_query", fmt.Sprintf("%s: expected one value", key))
		}
		var value any = texts[0]
		for _, boolean := range booleans {
			if key == boolean {
				parsed, err := webapi.ParseStrictBool(texts[0])
				if err != nil {
					return zero, apiFailure(400, "invalid_query", key+": "+err.Error())
				}
				value = bool(parsed)
			}
		}
		fields = append(fields, contract.Field{Name: key, Value: value})
	}
	data, err := contract.EncodeObject(fields)
	if err != nil {
		return zero, err
	}
	result, err := decode(data)
	if err != nil {
		return zero, apiFailure(400, "invalid_query", strings.TrimSpace(err.Error()))
	}
	return result, nil
}
