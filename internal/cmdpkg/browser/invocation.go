package browser

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/live"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/contract"
)

// Invoke executes a browser command and writes its completion output.
func (s *service) Invoke(
	ctx context.Context,
	invocation cmdsdk.InvocationContext[cmdproto.Invocation],
) (cmdproto.Completion, error) {
	operation, parseErr := browserproto.ParseOperation(invocation.Request.Operation, invocation.Request.Args)
	if errors.Is(parseErr, browserproto.ErrUnknownOperation) {
		return cmdproto.Completion{}, parseErr
	}
	browser, err := s.admit(invocation.Request.Context.Conversation)
	if err != nil {
		return cmdproto.Completion{}, err
	}
	defer browser.commands.Done()
	if _, ok := operation.(*browserproto.LiveInput); ok {
		return live.Serve(ctx, browser, invocation)
	}
	ctx, cancel := context.WithCancel(ctx)
	stopped := make(chan struct{})
	stop := context.AfterFunc(browser.released, func() {
		cancel()
		close(stopped)
	})
	defer func() {
		if !stop() {
			<-stopped
		}
		cancel()
	}()
	var bytes []byte
	if parseErr != nil {
		err = &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: parseErr.Error(), Cause: parseErr}
	} else {
		command, ok := operation.(browserproto.Input)
		if !ok {
			return cmdproto.Completion{}, fmt.Errorf(
				"%w %s",
				browserproto.ErrUnknownOperation,
				invocation.Request.Operation,
			)
		}
		bytes, err = s.invoke(ctx, browser, &invocation, command)
	}
	if err == nil {
		return cmdproto.Completion{}, invocation.Output.Stdout(ctx, bytes)
	}
	if ctx.Err() != nil {
		return cmdproto.Completion{}, ctx.Err()
	}
	return failure(ctx, browser, invocation, err)
}

func failure(
	ctx context.Context,
	browser *conversation,
	invocation cmdsdk.InvocationContext[cmdproto.Invocation],
	err error,
) (cmdproto.Completion, error) {
	code := cdp.ErrorCode(err)
	details := cdp.ErrorDetails(err)
	if details.Action == nil {
		action := browserproto.ActionProgress("not_started")
		details.Action = &action
	}
	if details.Tab == nil {
		readFailureTab(invocation.Request.Args, &details)
	}

	if code == "timeout" && details.Tab != nil {
		readDebuggingCallers(ctx, browser, invocation, &details)
	}

	bytes := []byte(cdp.RenderError(code, err.Error(), details))
	if invocation.Request.JSON != nil && *invocation.Request.JSON {
		var encodeErr error
		bytes, encodeErr = (browserproto.FailureDocument{
			Error: browserproto.BrowserFailure{Code: code, Message: err.Error(), Details: &details},
		}).MarshalJSON()
		if encodeErr != nil {
			return cmdproto.Completion{}, encodeErr
		}
	}
	exit := uint8(1)
	switch code {
	case "invalid_input":
		exit = 2
	case "cancelled":
		exit = 130
	}
	return cmdproto.Completion{ExitCode: exit}, invocation.Output.Stderr(ctx, bytes)
}

func (s *service) invoke(
	ctx context.Context,
	browser *conversation,
	invocation *cmdsdk.InvocationContext[cmdproto.Invocation],
	command browserproto.Input,
) ([]byte, error) {
	var bytes []byte
	deadline := time.Now().Add(command.Timeout())
	work, finish := context.WithDeadline(ctx, deadline)
	result, executeErr := s.execute(work, ctx, browser, invocation, command, deadline)
	err := executeErr
	if errors.Is(work.Err(), context.DeadlineExceeded) &&
		(cdp.ErrorCode(err) == "cancelled" || cdp.ErrorCode(err) == "timeout") {
		err = &cdp.BrowserError{
			Kind:    cdp.KindAction,
			Details: cdp.ErrorDetails(err),
			Cause:   &cdp.BrowserError{Kind: cdp.KindTimeout},
		}
	}
	finish()

	if err == nil {
		if result.png != nil {
			bytes = result.png
		} else {
			bytes, err = cdp.Render(
				command,
				result.json,
				invocation.Request.JSON != nil && *invocation.Request.JSON,
			)
		}
	}
	return bytes, err
}

// readFailureTab reads a refused tab identity for diagnostics, without dispatching it.
func readFailureTab(args []byte, details *browserproto.ErrorDetails) {
	// A refused object may still name a tab. This is diagnostic data only;
	// dispatch always uses the generated operation decoder above.
	fields, fieldErr := contract.ObjectFields(args)
	if fieldErr != nil {
		return
	}
	for _, field := range fields {
		if field.Name != "tab" {
			continue
		}
		raw, ok := field.Value.(json.RawMessage)
		if !ok || contract.IsNull(raw) {
			continue
		}
		if tab, textErr := contract.Decode[string](raw); textErr == nil {
			details.Tab = &tab
		}
	}
}

func readDebuggingCallers(
	ctx context.Context,
	browser *conversation,
	invocation cmdsdk.InvocationContext[cmdproto.Invocation],
	details *browserproto.ErrorDetails,
) {
	environment, _, lookupErr := browser.Running(ctx)
	if lookupErr == nil && environment != nil {
		var caller *uint64
		if agent, ok := invocation.Request.Context.Caller.(*cmdproto.AgentCaller); ok {
			caller = &agent.Number
		}
		callers := environment.DebuggingCallers(browserproto.TabID(*details.Tab), caller)
		if len(callers) > 0 {
			details.DebuggingCallers = &callers
		}
	}
}
