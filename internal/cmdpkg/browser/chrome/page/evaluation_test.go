package page

import (
	"encoding/json"
	"fmt"
	"strings"
	"testing"

	"github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp/cdptest"
)

// Tests Chrome's request and result boundary; no browser or clock wait (under 1 s).
func TestReadOnlyEvaluationBoundary(t *testing.T) {
	for _, tc := range []struct {
		name      string
		result    json.RawMessage
		exception *runtime.ExceptionDetails
		code      string
	}{
		{name: "ordered_unescaped_result", result: json.RawMessage(`"{\"z\":\"<>&\",\"a\":1}"`)},
		{name: "non_json", result: json.RawMessage(`"undefined"`), code: "unsupported_result"},
		{name: "wrong_result_type", result: json.RawMessage(`42`), code: "unsupported_result"},
		{
			name: "side_effect",
			exception: &runtime.ExceptionDetails{
				Text: "Uncaught",
				Exception: &runtime.RemoteObject{
					Type:        runtime.TypeObject,
					Description: "EvalError: Possible side-effect in debug-evaluate",
				},
			},
			code: "side_effect_rejected",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			expression := "document.title"
			executor := cdptest.NewExecutor(
				t,
				cdptest.Exchange{
					Method: runtime.CommandEvaluate,
					Params: runtime.Evaluate(fmt.Sprintf("%s(\n(%s)\n, %d)", scriptReadOnly, expression, maxEvalBytes)).
						WithThrowOnSideEffect(true).
						WithReturnByValue(true).
						WithAwaitPromise(false).
						WithTimeout(1000),
					Result: &runtime.EvaluateReturns{
						Result:           &runtime.RemoteObject{Type: runtime.TypeString, Value: tc.result},
						ExceptionDetails: tc.exception,
					},
				},
			)
			result, err := readOnly(t.Context(), executor, expression)
			if tc.code != "" {
				if err == nil || string(cdp.ErrorCode(err)) != tc.code {
					t.Fatalf("error=%v, want %s", err, tc.code)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if string(result) != `{"z":"<>&","a":1}` {
				t.Fatalf("wire bytes changed: %s", result)
			}
		})
	}
	t.Run("oversize_expression_is_not_dispatched", func(t *testing.T) {
		executor := cdptest.NewExecutor(t)
		_, err := readOnly(t.Context(), executor, strings.Repeat("x", maxEvalBytes+1))
		if err == nil || cdp.ErrorCode(err) != "unsupported_result" {
			t.Fatalf("error=%v", err)
		}
	})
}

func TestElementStateRejectsMalformedBrowserData(t *testing.T) {
	valid := `{"fillKind":"text","attached":true,"visible":true,"enabled":true,"checked":null,` +
		`"radio":false,"failed":null,"interceptor":null,"permanent":false,"x":1,"y":2}`
	for _, tc := range []struct{ name, body string }{
		{"missing", strings.Replace(valid, `"attached":true,`, "", 1)},
		{"null", strings.Replace(valid, `"visible":true`, `"visible":null`, 1)},
		{"unknown_kind", strings.Replace(valid, `"text"`, `"custom"`, 1)},
		{"duplicate", strings.Replace(valid, `"x":1`, `"x":1,"x":2`, 1)},
		{"extra", strings.Replace(valid, `"x":1`, `"extra":1,"x":1`, 1)},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var state elementState
			if err := json.Unmarshal([]byte(tc.body), &state); err == nil {
				t.Fatal("accepted corrupt state")
			}
		})
	}
	var state elementState
	if err := json.Unmarshal([]byte(valid), &state); err != nil {
		t.Fatal(err)
	}
	if !state.Visible || state.X != 1 || state.FillKind != "text" {
		t.Fatalf("state=%+v", state)
	}
}
