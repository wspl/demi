package browser

import (
	"context"
	"encoding/json"
	"errors"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/live"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
)

func (s *service) Invoke(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	operation, parseErr := browserop.ParseOperation(invocation.Request.Operation, invocation.Request.Args)
	var unknown *browserop.UnknownOperation
	var unserved *browserop.UnservedOperation
	if errors.As(parseErr, &unknown) || errors.As(parseErr, &unserved) {
		return commandwire.Completion{}, parseErr
	}
	b, err := s.admit(invocation.Request.Context.Conversation)
	if err != nil {
		return commandwire.Completion{}, err
	}
	defer b.commands.Done()
	if _, ok := operation.(*browserop.LiveInput); ok {
		return live.Serve(ctx, b, invocation)
	}
	ctx, cancel := context.WithCancel(ctx)
	stopped := make(chan struct{})
	stop := context.AfterFunc(b.released, func() {
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
		command, ok := operation.(browserop.Input)
		if !ok {
			return commandwire.Completion{}, &browserop.UnknownOperation{Name: invocation.Request.Operation}
		}
		deadline := time.Now().Add(command.Timeout())
		work, finish := context.WithDeadline(ctx, deadline)
		result, executeErr := s.execute(work, ctx, b, &invocation, command, deadline)
		err = executeErr
		if errors.Is(work.Err(), context.DeadlineExceeded) && (cdp.ErrorCode(err) == "cancelled" || cdp.ErrorCode(err) == "timeout") {
			err = &cdp.BrowserError{Kind: cdp.KindAction, Details: cdp.ErrorDetails(err), Cause: &cdp.BrowserError{Kind: cdp.KindTimeout}}
		}
		finish()

		if err == nil {
			if result.png != nil {
				bytes = result.png
			} else {
				bytes, err = cdp.Render(command, result.json, invocation.Request.JSON != nil && *invocation.Request.JSON)
			}
		}
	}
	if err == nil {
		return commandwire.Completion{}, invocation.Output.Stdout(ctx, bytes)
	}
	if ctx.Err() != nil {
		return commandwire.Completion{}, ctx.Err()
	}
	return failure(ctx, b, invocation, err)
}

func failure(ctx context.Context, b *conversation, invocation cmdsdk.InvocationContext[commandwire.Invocation], err error) (commandwire.Completion, error) {
	code := cdp.ErrorCode(err)
	details := cdp.ErrorDetails(err)
	if details.Action == nil {
		action := browserop.ActionProgress("not_started")
		details.Action = &action
	}
	if details.Tab == nil {
		// A refused object may still name a tab. This is diagnostic data only;
		// dispatch always uses the generated operation decoder above.
		if fields, fieldErr := contract.ObjectFields(invocation.Request.Args); fieldErr == nil {
			for _, field := range fields {
				if field.Name == "tab" {
					raw, ok := field.Value.(json.RawMessage)
					if !ok || contract.IsNull(raw) {
						continue
					}
					if tab, textErr := contract.Decode[string](raw); textErr == nil {
						details.Tab = &tab
					}
				}
			}
		}
	}

	if code == "timeout" && details.Tab != nil {
		environment, _, lookupErr := b.Running(ctx)
		if lookupErr == nil && environment != nil {
			var caller *uint64
			if agent, ok := invocation.Request.Context.Caller.(*commandwire.AgentCaller); ok {
				caller = &agent.Number
			}
			callers := environment.DebuggingCallers(browserop.TabID(*details.Tab), caller)
			if len(callers) > 0 {
				details.DebuggingCallers = &callers
			}
		}
	}
	bytes := []byte(cdp.RenderError(code, err.Error(), details))
	if invocation.Request.JSON != nil && *invocation.Request.JSON {
		var encodeErr error
		bytes, encodeErr = (browserop.FailureDocument{Error: browserop.BrowserFailure{Code: code, Message: err.Error(), Details: &details}}).MarshalJSON()
		if encodeErr != nil {
			return commandwire.Completion{}, encodeErr
		}
	}
	exit := uint8(1)
	switch code {
	case "invalid_input":
		exit = 2
	case "cancelled":
		exit = 130
	}
	return commandwire.Completion{ExitCode: exit}, invocation.Output.Stderr(ctx, bytes)
}
