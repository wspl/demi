package page

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
)

const maxEvalBytes = 1024 * 1024

// Evaluate evaluates expression read-only in the tab, holding its operation lock.
func Evaluate(ctx context.Context, tab *tabs.Tab, expression string, timeout time.Duration) (json.RawMessage, error) {
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindBusy}
	}
	defer checkout.Release()
	operation := tab.Operation(ctx, time.Now().Add(timeout))
	defer operation.Close()
	var result json.RawMessage
	err := operation.Run(ctx, func(work context.Context) error {
		var err error
		result, err = readOnly(work, tab.Page(), expression)
		return err
	})
	return result, err
}

// readOnly serializes the expression while Chrome enforces its read-only scope.
func readOnly(ctx context.Context, executor cdp.Executor, expression string) (json.RawMessage, error) {
	if len(expression) > maxEvalBytes {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "expression is too large"}
	}
	script := fmt.Sprintf("%s(\n(%s)\n, %d)", scriptReadOnly, expression, maxEvalBytes)
	object, exception, err := runtime.Evaluate(script).
		WithThrowOnSideEffect(true).
		WithReturnByValue(true).
		WithAwaitPromise(false).
		WithTimeout(1000).
		Do(protocol.WithExecutor(ctx, executor))
	if err != nil {
		return nil, err
	}
	if err = evaluationException(exception); err != nil {
		return nil, err
	}
	var value string
	if object == nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "evaluation returned no JSON value"}
	}
	if err = json.Unmarshal(object.Value, &value); err != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	if len(value) > maxEvalBytes {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "result is too large"}
	}
	if !json.Valid([]byte(value)) {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "evaluation returned invalid JSON"}
	}
	return json.RawMessage(value), nil
}

// evaluationException preserves Chrome's exception and its side-effect rejection.
func evaluationException(exception *runtime.ExceptionDetails) error {
	if exception == nil {
		return nil
	}
	if exception.Exception != nil &&
		strings.Contains(exception.Exception.Description, "Possible side-effect in debug-evaluate") {
		return &cdp.BrowserError{Kind: cdp.KindSideEffectRejected}
	}
	return &cdp.BrowserError{Kind: cdp.KindCDP, Message: exception.Error(), Cause: exception}
}

// targetedEvaluation binds observed nodes in one frame to a read-only expression.
func targetedEvaluation(
	ctx context.Context,
	expression string,
	elements []targetElement,
	all bool,
) (json.RawMessage, error) {
	if len(expression) > maxEvalBytes {
		return nil, &cdp.BrowserError{Kind: cdp.KindResultTooLarge}
	}
	if len(elements) == 0 {
		return nil, &cdp.BrowserError{Kind: cdp.KindTargetNotFound}
	}
	first := elements[0]
	if err := evaluationDocument(elements, first); err != nil {
		return nil, err
	}
	binding := "const element = args[0];"
	if all {
		binding = "const elements = args;"
	}
	script := fmt.Sprintf(
		"function(...args) { %s const document = this.ownerDocument || this; return (%s)((%s), %d); }",
		binding,
		scriptReadOnly,
		expression,
		maxEvalBytes,
	)
	args := make([]*runtime.CallArgument, 0, len(elements))
	for _, element := range elements {
		args = append(args, &runtime.CallArgument{ObjectID: element.object})
	}
	object, exception, err := runtime.CallFunctionOn(script).
		WithObjectID(first.object).
		WithArguments(args).
		WithThrowOnSideEffect(true).
		WithReturnByValue(true).
		WithAwaitPromise(false).
		Do(protocol.WithExecutor(ctx, first.page))
	if err != nil {
		return nil, err
	}
	if err = evaluationException(exception); err != nil {
		return nil, err
	}
	var value string
	if err = json.Unmarshal(object.Value, &value); err != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	if len(value) > maxEvalBytes {
		return nil, &cdp.BrowserError{Kind: cdp.KindResultTooLarge}
	}
	if !json.Valid([]byte(value)) {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "evaluation returned invalid JSON"}
	}
	return json.RawMessage(value), nil
}

func evaluationDocument(elements []targetElement, first targetElement) error {
	for _, element := range elements {
		same := element.page.TargetID() == first.page.TargetID() && len(element.frames) == len(first.frames)
		if same {
			for i, frame := range element.frames {
				same = same && frame.page.TargetID() == first.frames[i].page.TargetID() &&
					frame.backend == first.frames[i].backend
			}
		}
		if !same {
			return &cdp.BrowserError{
				Kind:    cdp.KindUnsupportedCapability,
				Message: "eval --all requires matches in one frame document; narrow with --frame or --within",
			}
		}
	}

	return nil
}
