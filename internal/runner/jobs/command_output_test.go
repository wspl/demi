package jobs

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/declare"
)

// Malformed output must tell the caller what the parser rejected, without
// reaching stdout or schema validation. This scenario uses no IO.
func TestJSONOutputReportsParserDetail(t *testing.T) {
	output := &commandOutput{schema: &declare.Schema{}, bytes: []byte(`{"answer": !}`)}
	err := output.finish(t.Context(), 0)
	var syntax *json.SyntaxError
	if !errors.As(err, &syntax) {
		t.Fatalf("missing parser cause: %v", err)
	}
	if !strings.HasPrefix(err.Error(), "--json output is not JSON: ") || !strings.Contains(err.Error(), syntax.Error()) || !strings.Contains(syntax.Error(), "invalid character '!'") {
		t.Fatalf("missing parser detail: %v", err)
	}
}
