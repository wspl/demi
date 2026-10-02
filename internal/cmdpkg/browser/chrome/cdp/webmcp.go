package cdp

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/page"
	"github.com/chromedp/cdproto/runtime"
	jsonv2 "github.com/go-json-experiment/json"
	"github.com/go-json-experiment/json/jsontext"
	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// ToolSet is a page-declared set of tools and its version-bound opaque handle.
// This is state held by the tab, not a second wire contract.
type ToolSet struct {
	Handle  string
	Entries []browserop.WebmcpTool
}

// WebMCPState belongs to the calling tab and is held under its operation gate.
// A nil Document means no document has yet been observed.
type WebMCPState struct {
	Document *protocol.LoaderID
	Key      *string
	Tools    *ToolSet
}

// evaluatePage reads a by-value result from Chrome without replacing execution scope.
func evaluatePage(ctx context.Context, executor Executor, script string, await bool) (json.RawMessage, error) {
	result, exception, err := runtime.Evaluate(script).WithAwaitPromise(await).WithReturnByValue(true).Do(protocol.WithExecutor(ctx, executor))
	if err != nil {
		return nil, err
	}
	if exception != nil {
		return nil, &BrowserError{Kind: KindInvalidResult, Message: exception.Text}
	}
	if result == nil || result.Value == nil {
		return nil, &BrowserError{Kind: KindInvalidResult, Message: "evaluation returned no JSON value"}
	}
	return json.RawMessage(result.Value), nil
}

// scriptString quotes browser-generated strings for insertion into JavaScript expressions.
func scriptString(value string) (string, error) {
	encoded, err := jsonv2.Marshal(value)
	if err != nil {
		return "", &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
	}
	return string(encoded), nil
}

// webMCPCapability asks this document whether Chrome exposes its native tool API.
func webMCPCapability(ctx context.Context, executor Executor) (browserop.Capability, error) {
	raw, err := evaluatePage(ctx, executor, "Boolean(document.modelContext && typeof document.modelContext.getTools === 'function' && typeof document.modelContext.executeTool === 'function')", false)
	if err != nil {
		return browserop.Capability{}, err
	}
	var available bool
	if err := jsonv2.Unmarshal(raw, &available); err != nil {
		return browserop.Capability{}, &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
	}
	capability := browserop.Capability{ID: "webmcp", Available: available}
	if available {
		schema := json.RawMessage(`{"help":"demi browser webmcp --help"}`)
		capability.Schema = &schema
	} else {
		reason := "this browser release or document does not expose document.modelContext"
		capability.Reason = &reason
	}
	return capability, nil
}

// ExecuteWebMCP discovers or calls native page tools under the caller's existing
// tab admission, validating declarations and values with offline JSON Schema.
// The caller supplies the document executor and tab-owned state.
func ExecuteWebMCP(ctx context.Context, operation *Operation, executor Executor, state *WebMCPState, tab browserop.TabID, command browserop.Operation) (json.RawMessage, error) {
	err := operation.Run(ctx, func(work context.Context) error {
		capability, err := webMCPCapability(work, executor)
		if err != nil {
			return err
		}
		if !capability.Available {
			return &BrowserError{Kind: KindUnsupportedCapability, Message: *capability.Reason}
		}
		tree, err := page.GetFrameTree().Do(protocol.WithExecutor(work, executor))
		if err != nil {
			return err
		}
		document := tree.Frame.LoaderID
		if state.Document == nil || *state.Document != document {
			state.Document = &document
			state.Key = nil
			state.Tools = nil
		}
		if state.Key == nil {
			key, err := Fresh("webmcp")
			if err != nil {
				return err
			}
			state.Key = &key
		}
		keyLiteral, err := scriptString(*state.Key)
		if err != nil {
			return err
		}
		script := fmt.Sprintf(`(() => {
   const key = %s;
   if (globalThis[key]) return true;
   const context = document.modelContext;
   const state = {version: 0, current: null, calls: new Map(), context};
   Object.defineProperty(globalThis, key, {value: state});
   context.addEventListener('toolchange', () => { state.version++; state.current = null; });
   return true;
  })()`, keyLiteral)
		raw, err := evaluatePage(work, executor, script, false)
		if err != nil {
			return err
		}
		var installed bool
		if err := jsonv2.Unmarshal(raw, &installed); err != nil {
			return &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
		}
		if !installed {
			return &BrowserError{Kind: KindInvalidResult, Message: "WebMCP observer was not installed"}
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	key, err := scriptString(*state.Key)
	if err != nil {
		return nil, err
	}
	if _, ok := command.(*browserop.WebmcpListInput); ok {
		handle, err := Fresh("tools")
		if err != nil {
			return nil, err
		}
		handleLiteral, err := scriptString(handle)
		if err != nil {
			return nil, err
		}
		script := fmt.Sprintf(`(async () => {
   const state = globalThis[%s];
   const version = state.version;
   const tools = await state.context.getTools();
   if (version !== state.version) return {status:'stale'};
   const entries = tools.map(tool => ({name:tool.name, description:tool.description, inputSchema:JSON.parse(tool.inputSchema)}));
   const handle = state.current?.version === version ? state.current.handle : %s;
   state.current = {version, tools, handle};
   return {status:'ready', entries, handle};
  })()`, key, handleLiteral)
		var raw json.RawMessage
		err = operation.Run(ctx, func(work context.Context) error {
			var err error
			raw, err = evaluatePage(work, executor, script, true)
			return err
		})
		if err != nil {
			return nil, err
		}
		var snapshot struct {
			Status  string           `json:"status"`
			Entries []jsontext.Value `json:"entries"`
			Handle  *string          `json:"handle"`
		}
		if err := jsonv2.Unmarshal(raw, &snapshot, jsonv2.RejectUnknownMembers(true)); err != nil {
			return nil, &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
		}
		if snapshot.Status == "stale" && snapshot.Entries == nil && snapshot.Handle == nil {
			return nil, &BrowserError{Kind: KindStaleTools}
		}
		if snapshot.Status != "ready" || snapshot.Entries == nil || snapshot.Handle == nil {
			return nil, &BrowserError{Kind: KindInvalidResult, Message: "invalid WebMCP discovery result"}
		}
		entries := make([]browserop.WebmcpTool, 0, len(snapshot.Entries))
		for _, raw := range snapshot.Entries {
			entry, err := browserop.DecodeWebmcpTool(raw)
			if err != nil {
				return nil, &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
			}
			if _, err := pageSchema(entry.InputSchema); err != nil {
				return nil, &BrowserError{Kind: KindInvalidResult, Message: "invalid tool input schema: " + err.Error(), Cause: err}
			}
			if entry.OutputSchema != nil {
				if _, err := pageSchema(*entry.OutputSchema); err != nil {
					return nil, &BrowserError{Kind: KindInvalidResult, Message: "invalid tool output schema: " + err.Error(), Cause: err}
				}
			}
			entries = append(entries, entry)
		}
		result, err := Value(browserop.WebmcpListResult{Tools: *snapshot.Handle, Entries: entries})
		if err != nil {
			return nil, err
		}
		if len(result) > browserop.InlineBytes {
			return nil, &BrowserError{Kind: KindResultTooLarge}
		}
		state.Tools = &ToolSet{Handle: *snapshot.Handle, Entries: entries}
		return result, nil
	}
	input, ok := command.(*browserop.WebmcpCallInput)
	if !ok {
		return nil, &BrowserError{Kind: KindConfiguration, Message: "WebMCP dispatch accepts only list/call"}
	}
	if state.Tools == nil || state.Tools.Handle != input.Tools {
		return nil, &BrowserError{Kind: KindStaleTools}
	}
	var entry *browserop.WebmcpTool
	count := uint(0)
	for i := range state.Tools.Entries {
		if state.Tools.Entries[i].Name == input.Tool {
			entry = &state.Tools.Entries[i]
			count++
		}
	}
	if count == 0 {
		return nil, &BrowserError{Kind: KindTargetNotFound}
	}
	if count > 1 {
		return nil, &BrowserError{Kind: KindAmbiguous, Count: count}
	}
	arguments, err := jsonschema.UnmarshalJSON(bytes.NewBufferString(input.Arguments))
	if err != nil {
		return nil, &BrowserError{Kind: KindConfiguration, Message: err.Error(), Cause: err}
	}
	schema, err := pageSchema(entry.InputSchema)
	if err != nil {
		return nil, &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
	}
	if err := schema.Validate(arguments); err != nil {
		return nil, &BrowserError{Kind: KindConfiguration, Message: err.Error(), Cause: err}
	}
	call, err := Fresh("toolcall")
	if err != nil {
		return nil, err
	}
	callString, err := scriptString(call)
	if err != nil {
		return nil, err
	}
	toolsLiteral, err := scriptString(input.Tools)
	if err != nil {
		return nil, err
	}
	toolLiteral, err := scriptString(input.Tool)
	if err != nil {
		return nil, err
	}
	argumentsLiteral, err := scriptString(input.Arguments)
	if err != nil {
		return nil, err
	}
	script := fmt.Sprintf(`(async () => {
  const state = globalThis[%s];
  const set = state?.current;
  if (!set || set.handle !== %s || set.version !== state.version) return {status:'stale'};
  const tool = set.tools.find(tool => tool.name === %s);
  if (!tool) return {status:'stale'};
  const controller = new AbortController();
  state.calls.set(%s, controller);
  try {
   const result = await state.context.executeTool(tool, %s, {signal:controller.signal});
   return {status:'completed', result: result === null ? null : JSON.parse(result)};
  } finally { state.calls.delete(%s); }
 })()`, key, toolsLiteral, toolLiteral, callString, argumentsLiteral, callString)
	var result json.RawMessage
	err = operation.Run(ctx, func(work context.Context) error {
		operation.BeginInput()
		raw, err := evaluatePage(work, executor, script, true)
		if err != nil {
			return err
		}
		var reply struct {
			Status string         `json:"status"`
			Result jsontext.Value `json:"result"`
		}
		if err := jsonv2.Unmarshal(raw, &reply, jsonv2.RejectUnknownMembers(true)); err != nil {
			return &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
		}
		if reply.Status == "stale" && reply.Result == nil {
			operation.InputNotDelivered()
			return &BrowserError{Kind: KindStaleTools}
		}
		if reply.Status != "completed" || reply.Result == nil {
			return &BrowserError{Kind: KindInvalidResult, Message: "invalid WebMCP call result"}
		}
		operation.CompleteInput()
		if entry.OutputSchema != nil {
			schema, err := pageSchema(*entry.OutputSchema)
			if err != nil {
				return &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
			}
			value, err := jsonschema.UnmarshalJSON(bytes.NewReader(reply.Result))
			if err == nil {
				err = schema.Validate(value)
			}
			if err != nil {
				return &BrowserError{Kind: KindInvalidResult, Message: err.Error(), Cause: err}
			}
		}
		result, err = Value(browserop.WebmcpCallResult{Name: input.Tool, Result: json.RawMessage(reply.Result)})
		return err
	})
	cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), ControlTimeout)
	defer cancel()
	_, cleanupErr := evaluatePage(cleanup, executor, fmt.Sprintf(`(() => { const state = globalThis[%s]; const call = state?.calls.get(%s); if (call) { call.abort(); state.calls.delete(%s); } return true; })()`, key, callString, callString), false)
	if ErrorCode(cleanupErr) == "browser_lost" || ErrorCode(cleanupErr) == "tab_not_found" {
		cleanupErr = nil
	}
	if err = AfterCleanup(err, cleanupErr); err != nil {
		return nil, operation.Failure(err, string(tab), nil)
	}
	return result, nil
}
