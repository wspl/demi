package browserop

import (
	"errors"
	"fmt"
	"iter"
	"strings"
	"time"
)

// Operation is a checked invocation of the demi.browser package.
type Operation interface {
	operation()
}

// Input answers the scheduling and targeting questions shared by browser commands.
type Input interface {
	Operation
	// OperationName is the name without the browser. prefix.
	OperationName() string
	// Timeout returns the whole operation deadline.
	Timeout() time.Duration
	// DefaultTimeoutMS returns the deadline used when the input names none.
	DefaultTimeoutMS() uint64
	// TabID returns the targeted tab, or nil for an untargeted command.
	TabID() *TabID
	// ElementTarget returns the element target carried by the input.
	ElementTarget() *BrowserTarget
	// WaitURLPattern returns the URL glob to wait for after input.
	WaitURLPattern() *string
}

// UnknownOperation is a name outside this package.
type UnknownOperation struct {
	// Name is the refused operation name.
	Name string
}

// Error returns the refused operation name.
func (e *UnknownOperation) Error() string { return fmt.Sprintf("unknown operation %s", e.Name) }

// UnservedOperation is a browser operation this package does not serve.
type UnservedOperation struct {
	// Name is the unserved operation name.
	Name string
}

// Error returns the unserved operation name.
func (e *UnservedOperation) Error() string { return fmt.Sprintf("unknown operation %s", e.Name) }

// InvalidInput wraps the reason an operation's arguments were refused.
type InvalidInput struct {
	// Err is the argument validation failure.
	Err error
}

// Error returns the argument validation failure.
func (e *InvalidInput) Error() string { return e.Err.Error() }

// Unwrap returns the argument validation failure.
func (e *InvalidInput) Unwrap() error { return e.Err }

// ParseOperation decodes the named invocation through its generated input decoder.
func ParseOperation(name string, args []byte) (Operation, error) {
	if name == LiveOperation {
		v, err := DecodeLiveInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	}
	if !strings.HasPrefix(name, Prefix) {
		return nil, &UnknownOperation{Name: name}
	}
	return ParseInput(strings.TrimPrefix(name, Prefix), args)
}

// ParseInput decodes a browser operation named without its browser. prefix.
func ParseInput(name string, args []byte) (Input, error) {
	switch name {
	case "open", "tabs", "info", "close":
		return parseTabInput(name, args)
	case "goto", "back", "forward", "reload", "history":
		return parseNavigationInput(name, args)
	case "inspect", "find", "read", "screenshot", "probe", "wait":
		return parseObservationInput(name, args)
	case "click", "move", "drag", "scroll":
		return parsePointerInput(name, args)
	case "fill", "type", "key", "check", "select", "select-text":
		return parseEditingInput(name, args)
	case "upload", "download", "clipboard.write", "clipboard.read":
		return parseTransferInput(name, args)
	case "eval", "logs", "viewport.set", "viewport.reset", "dialog.inspect", "dialog.accept", "dialog.dismiss":
		return parsePageInput(name, args)
	case "cdp.targets", "cdp.detach", "cdp.send", "cdp.events":
		return parseDebuggingInput(name, args)
	case "content.read", "content.fetch", "assets.list", "assets.export", "capabilities", "webmcp.list", "webmcp.call":
		return parseContentInput(name, args)
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

func parseTabInput(name string, args []byte) (Input, error) {
	switch name {
	case "open":
		v, err := DecodeOpenInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "tabs":
		v, err := DecodeTabsInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "info":
		v, err := DecodeInfoInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "close":
		v, err := DecodeCloseInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

func parseNavigationInput(name string, args []byte) (Input, error) {
	switch name {
	case "goto":
		v, err := DecodeGotoInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "back":
		v, err := DecodeBackInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "forward":
		v, err := DecodeForwardInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "reload":
		v, err := DecodeReloadInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "history":
		v, err := DecodeHistoryInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

func parseObservationInput(name string, args []byte) (Input, error) {
	switch name {
	case "inspect":
		v, err := DecodeInspectInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "find":
		v, err := DecodeFindInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "read":
		v, err := DecodeReadInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "screenshot":
		v, err := DecodeScreenshotInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "probe":
		v, err := DecodeProbeInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "wait":
		v, err := DecodeWaitInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

func parsePointerInput(name string, args []byte) (Input, error) {
	switch name {
	case "click":
		v, err := DecodeClickInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "move":
		v, err := DecodeMoveInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "drag":
		v, err := DecodeDragInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "scroll":
		v, err := DecodeScrollInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

func parseEditingInput(name string, args []byte) (Input, error) {
	switch name {
	case "fill":
		v, err := DecodeFillInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "type":
		v, err := DecodeTypeInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "key":
		v, err := DecodeKeyInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "check":
		v, err := DecodeCheckInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "select":
		v, err := DecodeSelectInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "select-text":
		v, err := DecodeSelectTextInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

func parseTransferInput(name string, args []byte) (Input, error) {
	switch name {
	case "upload":
		v, err := DecodeUploadInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "download":
		v, err := DecodeDownloadInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "clipboard.write":
		v, err := DecodeClipboardWriteInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "clipboard.read":
		v, err := DecodeClipboardReadInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

func parsePageInput(name string, args []byte) (Input, error) {
	switch name {
	case "eval":
		v, err := DecodeEvalInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "logs":
		v, err := DecodeLogsInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "viewport.set":
		v, err := DecodeViewportSetInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "viewport.reset":
		v, err := DecodeViewportResetInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "dialog.inspect":
		v, err := DecodeDialogInspectInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "dialog.accept":
		v, err := DecodeDialogAcceptInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "dialog.dismiss":
		v, err := DecodeDialogDismissInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

func parseDebuggingInput(name string, args []byte) (Input, error) {
	switch name {
	case "cdp.targets":
		v, err := DecodeCdpTargetsInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "cdp.detach":
		v, err := DecodeCdpDetachInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "cdp.send":
		v, err := DecodeCdpSendInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "cdp.events":
		v, err := DecodeCdpEventsInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

func parseContentInput(name string, args []byte) (Input, error) {
	switch name {
	case "content.read":
		v, err := DecodeContentReadInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "content.fetch":
		v, err := DecodeContentFetchInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "assets.list":
		v, err := DecodeAssetsListInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "assets.export":
		v, err := DecodeAssetsExportInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "capabilities":
		v, err := DecodeCapabilitiesInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "webmcp.list":
		v, err := DecodeWebmcpListInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	case "webmcp.call":
		v, err := DecodeWebmcpCallInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
	default:
		return nil, &UnservedOperation{Name: Prefix + name}
	}
}

// Operations returns the browser commands in descriptor order.
func Operations() []string {
	return []string{
		"browser.open",
		"browser.tabs",
		"browser.info",
		"browser.goto",
		"browser.back",
		"browser.forward",
		"browser.reload",
		"browser.history",
		"browser.close",
		"browser.inspect",
		"browser.find",
		"browser.read",
		"browser.screenshot",
		"browser.probe",
		"browser.click",
		"browser.move",
		"browser.drag",
		"browser.scroll",
		"browser.fill",
		"browser.type",
		"browser.key",
		"browser.check",
		"browser.select",
		"browser.select-text",
		"browser.wait",
		"browser.upload",
		"browser.download",
		"browser.clipboard.write",
		"browser.clipboard.read",
		"browser.eval",
		"browser.logs",
		"browser.viewport.set",
		"browser.viewport.reset",
		"browser.dialog.inspect",
		"browser.dialog.accept",
		"browser.dialog.dismiss",
		"browser.cdp.targets",
		"browser.cdp.detach",
		"browser.cdp.send",
		"browser.cdp.events",
		"browser.content.read",
		"browser.content.fetch",
		"browser.assets.list",
		"browser.assets.export",
		"browser.capabilities",
		"browser.webmcp.list",
		"browser.webmcp.call",
	}
}

// OperationNames returns every served operation, including the live view.
func OperationNames() []string { return append(Operations(), LiveOperation) }

func (*LiveInput) operation() {}

// FullName returns the live operation name in the package descriptor.
func (*LiveInput) FullName() string { return LiveOperation }

func (*OpenInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*OpenInput) OperationName() string { return "open" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*OpenInput) DefaultTimeoutMS() uint64 { return MaxTimeoutMS }

// Timeout is the whole operation's deadline.
func (oi *OpenInput) Timeout() time.Duration {
	ms := oi.DefaultTimeoutMS()
	if oi.TimeoutMS != nil {
		ms = *oi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (oi *OpenInput) TabID() *TabID { return nil }

// WaitURLPattern is the URL glob the action waits for after its input.
func (oi *OpenInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*OpenInput) ElementTarget() *BrowserTarget { return nil }

func (*TabsInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*TabsInput) OperationName() string { return "tabs" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*TabsInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ti *TabsInput) Timeout() time.Duration {
	ms := ti.DefaultTimeoutMS()
	if ti.TimeoutMS != nil {
		ms = *ti.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ti *TabsInput) TabID() *TabID { return nil }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ti *TabsInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*TabsInput) ElementTarget() *BrowserTarget { return nil }

func (*InfoInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*InfoInput) OperationName() string { return "info" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*InfoInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ii *InfoInput) Timeout() time.Duration {
	ms := ii.DefaultTimeoutMS()
	if ii.TimeoutMS != nil {
		ms = *ii.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ii *InfoInput) TabID() *TabID { return new(ii.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ii *InfoInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*InfoInput) ElementTarget() *BrowserTarget { return nil }

func (*GotoInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*GotoInput) OperationName() string { return "goto" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*GotoInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (gi *GotoInput) Timeout() time.Duration {
	ms := gi.DefaultTimeoutMS()
	if gi.TimeoutMS != nil {
		ms = *gi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (gi *GotoInput) TabID() *TabID { return new(gi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (gi *GotoInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*GotoInput) ElementTarget() *BrowserTarget { return nil }

func (*BackInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*BackInput) OperationName() string { return "back" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*BackInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (bi *BackInput) Timeout() time.Duration {
	ms := bi.DefaultTimeoutMS()
	if bi.TimeoutMS != nil {
		ms = *bi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (bi *BackInput) TabID() *TabID { return new(bi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (bi *BackInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*BackInput) ElementTarget() *BrowserTarget { return nil }

func (*ForwardInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ForwardInput) OperationName() string { return "forward" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ForwardInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (fi *ForwardInput) Timeout() time.Duration {
	ms := fi.DefaultTimeoutMS()
	if fi.TimeoutMS != nil {
		ms = *fi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (fi *ForwardInput) TabID() *TabID { return new(fi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (fi *ForwardInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ForwardInput) ElementTarget() *BrowserTarget { return nil }

func (*ReloadInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ReloadInput) OperationName() string { return "reload" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ReloadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ri *ReloadInput) Timeout() time.Duration {
	ms := ri.DefaultTimeoutMS()
	if ri.TimeoutMS != nil {
		ms = *ri.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ri *ReloadInput) TabID() *TabID { return new(ri.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ri *ReloadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ReloadInput) ElementTarget() *BrowserTarget { return nil }

func (*HistoryInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*HistoryInput) OperationName() string { return "history" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*HistoryInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (hi *HistoryInput) Timeout() time.Duration {
	ms := hi.DefaultTimeoutMS()
	if hi.TimeoutMS != nil {
		ms = *hi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (hi *HistoryInput) TabID() *TabID { return new(hi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (hi *HistoryInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*HistoryInput) ElementTarget() *BrowserTarget { return nil }

func (*CloseInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*CloseInput) OperationName() string { return "close" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CloseInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ci *CloseInput) Timeout() time.Duration {
	ms := ci.DefaultTimeoutMS()
	if ci.TimeoutMS != nil {
		ms = *ci.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ci *CloseInput) TabID() *TabID { return new(ci.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ci *CloseInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CloseInput) ElementTarget() *BrowserTarget { return nil }

func (*InspectInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*InspectInput) OperationName() string { return "inspect" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*InspectInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ii *InspectInput) Timeout() time.Duration {
	ms := ii.DefaultTimeoutMS()
	if ii.TimeoutMS != nil {
		ms = *ii.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ii *InspectInput) TabID() *TabID { return new(ii.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ii *InspectInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*InspectInput) ElementTarget() *BrowserTarget { return nil }

func (*FindInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*FindInput) OperationName() string { return "find" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*FindInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (fi *FindInput) Timeout() time.Duration {
	ms := fi.DefaultTimeoutMS()
	if fi.TimeoutMS != nil {
		ms = *fi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (fi *FindInput) TabID() *TabID { return new(fi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (fi *FindInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (fi *FindInput) ElementTarget() *BrowserTarget {
	target := fi.clone()
	return &target
}

func (*ReadInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ReadInput) OperationName() string { return "read" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ReadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ri *ReadInput) Timeout() time.Duration {
	ms := ri.DefaultTimeoutMS()
	if ri.TimeoutMS != nil {
		ms = *ri.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ri *ReadInput) TabID() *TabID { return new(ri.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ri *ReadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (ri *ReadInput) ElementTarget() *BrowserTarget {
	target := ri.clone()
	return &target
}

func (*ScreenshotInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ScreenshotInput) OperationName() string { return "screenshot" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ScreenshotInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (si *ScreenshotInput) Timeout() time.Duration {
	ms := si.DefaultTimeoutMS()
	if si.TimeoutMS != nil {
		ms = *si.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (si *ScreenshotInput) TabID() *TabID { return new(si.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (si *ScreenshotInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ScreenshotInput) ElementTarget() *BrowserTarget { return nil }

func (*ProbeInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ProbeInput) OperationName() string { return "probe" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ProbeInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (pi *ProbeInput) Timeout() time.Duration {
	ms := pi.DefaultTimeoutMS()
	if pi.TimeoutMS != nil {
		ms = *pi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (pi *ProbeInput) TabID() *TabID { return new(pi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (pi *ProbeInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ProbeInput) ElementTarget() *BrowserTarget { return nil }

func (*ClickInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ClickInput) OperationName() string { return "click" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ClickInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ci *ClickInput) Timeout() time.Duration {
	ms := ci.DefaultTimeoutMS()
	if ci.TimeoutMS != nil {
		ms = *ci.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ci *ClickInput) TabID() *TabID { return new(ci.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ci *ClickInput) WaitURLPattern() *string {
	if ci.WaitURL == nil {
		return nil
	}
	return new(*ci.WaitURL)
}

// ElementTarget is the element target the input carries.
func (ci *ClickInput) ElementTarget() *BrowserTarget {
	target := ci.clone()
	return &target
}

func (*MoveInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*MoveInput) OperationName() string { return "move" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*MoveInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (mi *MoveInput) Timeout() time.Duration {
	ms := mi.DefaultTimeoutMS()
	if mi.TimeoutMS != nil {
		ms = *mi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (mi *MoveInput) TabID() *TabID { return new(mi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (mi *MoveInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (mi *MoveInput) ElementTarget() *BrowserTarget {
	target := mi.clone()
	return &target
}

func (*DragInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*DragInput) OperationName() string { return "drag" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DragInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (di *DragInput) Timeout() time.Duration {
	ms := di.DefaultTimeoutMS()
	if di.TimeoutMS != nil {
		ms = *di.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (di *DragInput) TabID() *TabID { return new(di.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (di *DragInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*DragInput) ElementTarget() *BrowserTarget { return nil }

func (*ScrollInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ScrollInput) OperationName() string { return "scroll" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ScrollInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (si *ScrollInput) Timeout() time.Duration {
	ms := si.DefaultTimeoutMS()
	if si.TimeoutMS != nil {
		ms = *si.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (si *ScrollInput) TabID() *TabID { return new(si.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (si *ScrollInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (si *ScrollInput) ElementTarget() *BrowserTarget {
	target := si.clone()
	return &target
}

func (*FillInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*FillInput) OperationName() string { return "fill" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*FillInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (fi *FillInput) Timeout() time.Duration {
	ms := fi.DefaultTimeoutMS()
	if fi.TimeoutMS != nil {
		ms = *fi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (fi *FillInput) TabID() *TabID { return new(fi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (fi *FillInput) WaitURLPattern() *string {
	if fi.WaitURL == nil {
		return nil
	}
	return new(*fi.WaitURL)
}

// ElementTarget is the element target the input carries.
func (fi *FillInput) ElementTarget() *BrowserTarget {
	target := fi.clone()
	return &target
}

func (*TypeInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*TypeInput) OperationName() string { return "type" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*TypeInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ti *TypeInput) Timeout() time.Duration {
	ms := ti.DefaultTimeoutMS()
	if ti.TimeoutMS != nil {
		ms = *ti.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ti *TypeInput) TabID() *TabID { return new(ti.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ti *TypeInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (ti *TypeInput) ElementTarget() *BrowserTarget {
	target := ti.clone()
	return &target
}

func (*KeyInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*KeyInput) OperationName() string { return "key" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*KeyInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ki *KeyInput) Timeout() time.Duration {
	ms := ki.DefaultTimeoutMS()
	if ki.TimeoutMS != nil {
		ms = *ki.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ki *KeyInput) TabID() *TabID { return new(ki.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ki *KeyInput) WaitURLPattern() *string {
	if ki.WaitURL == nil {
		return nil
	}
	return new(*ki.WaitURL)
}

// ElementTarget is the element target the input carries.
func (ki *KeyInput) ElementTarget() *BrowserTarget {
	target := ki.clone()
	return &target
}

func (*CheckInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*CheckInput) OperationName() string { return "check" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CheckInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ci *CheckInput) Timeout() time.Duration {
	ms := ci.DefaultTimeoutMS()
	if ci.TimeoutMS != nil {
		ms = *ci.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ci *CheckInput) TabID() *TabID { return new(ci.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ci *CheckInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (ci *CheckInput) ElementTarget() *BrowserTarget {
	target := ci.clone()
	return &target
}

func (*SelectInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*SelectInput) OperationName() string { return "select" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*SelectInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (si *SelectInput) Timeout() time.Duration {
	ms := si.DefaultTimeoutMS()
	if si.TimeoutMS != nil {
		ms = *si.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (si *SelectInput) TabID() *TabID { return new(si.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (si *SelectInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (si *SelectInput) ElementTarget() *BrowserTarget {
	target := si.clone()
	return &target
}

func (*SelectTextInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*SelectTextInput) OperationName() string { return "select-text" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*SelectTextInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (sti *SelectTextInput) Timeout() time.Duration {
	ms := sti.DefaultTimeoutMS()
	if sti.TimeoutMS != nil {
		ms = *sti.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (sti *SelectTextInput) TabID() *TabID { return new(sti.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (sti *SelectTextInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (sti *SelectTextInput) ElementTarget() *BrowserTarget {
	target := sti.clone()
	return &target
}

func (*WaitInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*WaitInput) OperationName() string { return "wait" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*WaitInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (wi *WaitInput) Timeout() time.Duration {
	ms := wi.DefaultTimeoutMS()
	if wi.TimeoutMS != nil {
		ms = *wi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (wi *WaitInput) TabID() *TabID { return new(wi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (wi *WaitInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (wi *WaitInput) ElementTarget() *BrowserTarget {
	target := wi.clone()
	return &target
}

func (*UploadInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*UploadInput) OperationName() string { return "upload" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*UploadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ui *UploadInput) Timeout() time.Duration {
	ms := ui.DefaultTimeoutMS()
	if ui.TimeoutMS != nil {
		ms = *ui.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ui *UploadInput) TabID() *TabID { return new(ui.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ui *UploadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (ui *UploadInput) ElementTarget() *BrowserTarget {
	target := ui.clone()
	return &target
}

func (*DownloadInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*DownloadInput) OperationName() string { return "download" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DownloadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (di *DownloadInput) Timeout() time.Duration {
	ms := di.DefaultTimeoutMS()
	if di.TimeoutMS != nil {
		ms = *di.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (di *DownloadInput) TabID() *TabID { return new(di.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (di *DownloadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (di *DownloadInput) ElementTarget() *BrowserTarget {
	target := di.clone()
	return &target
}

func (*ClipboardWriteInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ClipboardWriteInput) OperationName() string { return "clipboard.write" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ClipboardWriteInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (cwi *ClipboardWriteInput) Timeout() time.Duration {
	ms := cwi.DefaultTimeoutMS()
	if cwi.TimeoutMS != nil {
		ms = *cwi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (cwi *ClipboardWriteInput) TabID() *TabID { return new(cwi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (cwi *ClipboardWriteInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ClipboardWriteInput) ElementTarget() *BrowserTarget { return nil }

func (*ClipboardReadInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ClipboardReadInput) OperationName() string { return "clipboard.read" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ClipboardReadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (cri *ClipboardReadInput) Timeout() time.Duration {
	ms := cri.DefaultTimeoutMS()
	if cri.TimeoutMS != nil {
		ms = *cri.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (cri *ClipboardReadInput) TabID() *TabID { return new(cri.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (cri *ClipboardReadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ClipboardReadInput) ElementTarget() *BrowserTarget { return nil }

func (*EvalInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*EvalInput) OperationName() string { return "eval" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*EvalInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ei *EvalInput) Timeout() time.Duration {
	ms := ei.DefaultTimeoutMS()
	if ei.TimeoutMS != nil {
		ms = *ei.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ei *EvalInput) TabID() *TabID { return new(ei.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ei *EvalInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (ei *EvalInput) ElementTarget() *BrowserTarget {
	target := ei.clone()
	return &target
}

func (*LogsInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*LogsInput) OperationName() string { return "logs" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*LogsInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (li *LogsInput) Timeout() time.Duration {
	ms := li.DefaultTimeoutMS()
	if li.TimeoutMS != nil {
		ms = *li.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (li *LogsInput) TabID() *TabID { return new(li.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (li *LogsInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*LogsInput) ElementTarget() *BrowserTarget { return nil }

func (*ViewportSetInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ViewportSetInput) OperationName() string { return "viewport.set" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ViewportSetInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (vsi *ViewportSetInput) Timeout() time.Duration {
	ms := vsi.DefaultTimeoutMS()
	if vsi.TimeoutMS != nil {
		ms = *vsi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (vsi *ViewportSetInput) TabID() *TabID { return new(vsi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (vsi *ViewportSetInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ViewportSetInput) ElementTarget() *BrowserTarget { return nil }

func (*ViewportResetInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ViewportResetInput) OperationName() string { return "viewport.reset" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ViewportResetInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (vri *ViewportResetInput) Timeout() time.Duration {
	ms := vri.DefaultTimeoutMS()
	if vri.TimeoutMS != nil {
		ms = *vri.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (vri *ViewportResetInput) TabID() *TabID { return new(vri.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (vri *ViewportResetInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ViewportResetInput) ElementTarget() *BrowserTarget { return nil }

func (*DialogInspectInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*DialogInspectInput) OperationName() string { return "dialog.inspect" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DialogInspectInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (dii *DialogInspectInput) Timeout() time.Duration {
	ms := dii.DefaultTimeoutMS()
	if dii.TimeoutMS != nil {
		ms = *dii.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (dii *DialogInspectInput) TabID() *TabID { return new(dii.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (dii *DialogInspectInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*DialogInspectInput) ElementTarget() *BrowserTarget { return nil }

func (*DialogAcceptInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*DialogAcceptInput) OperationName() string { return "dialog.accept" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DialogAcceptInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (dai *DialogAcceptInput) Timeout() time.Duration {
	ms := dai.DefaultTimeoutMS()
	if dai.TimeoutMS != nil {
		ms = *dai.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (dai *DialogAcceptInput) TabID() *TabID { return new(dai.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (dai *DialogAcceptInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*DialogAcceptInput) ElementTarget() *BrowserTarget { return nil }

func (*DialogDismissInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*DialogDismissInput) OperationName() string { return "dialog.dismiss" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DialogDismissInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ddi *DialogDismissInput) Timeout() time.Duration {
	ms := ddi.DefaultTimeoutMS()
	if ddi.TimeoutMS != nil {
		ms = *ddi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ddi *DialogDismissInput) TabID() *TabID { return new(ddi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ddi *DialogDismissInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*DialogDismissInput) ElementTarget() *BrowserTarget { return nil }

func (*CdpTargetsInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*CdpTargetsInput) OperationName() string { return "cdp.targets" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CdpTargetsInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (cti *CdpTargetsInput) Timeout() time.Duration {
	ms := cti.DefaultTimeoutMS()
	if cti.TimeoutMS != nil {
		ms = *cti.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (cti *CdpTargetsInput) TabID() *TabID { return new(cti.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (cti *CdpTargetsInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CdpTargetsInput) ElementTarget() *BrowserTarget { return nil }

func (*CdpDetachInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*CdpDetachInput) OperationName() string { return "cdp.detach" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CdpDetachInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (cdi *CdpDetachInput) Timeout() time.Duration {
	ms := cdi.DefaultTimeoutMS()
	if cdi.TimeoutMS != nil {
		ms = *cdi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (cdi *CdpDetachInput) TabID() *TabID { return new(cdi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (cdi *CdpDetachInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CdpDetachInput) ElementTarget() *BrowserTarget { return nil }

func (*CdpSendInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*CdpSendInput) OperationName() string { return "cdp.send" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CdpSendInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (csi *CdpSendInput) Timeout() time.Duration {
	ms := csi.DefaultTimeoutMS()
	if csi.TimeoutMS != nil {
		ms = *csi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (csi *CdpSendInput) TabID() *TabID { return new(csi.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (csi *CdpSendInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CdpSendInput) ElementTarget() *BrowserTarget { return nil }

func (*CdpEventsInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*CdpEventsInput) OperationName() string { return "cdp.events" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CdpEventsInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (cei *CdpEventsInput) Timeout() time.Duration {
	ms := cei.DefaultTimeoutMS()
	if cei.TimeoutMS != nil {
		ms = *cei.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (cei *CdpEventsInput) TabID() *TabID { return new(cei.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (cei *CdpEventsInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CdpEventsInput) ElementTarget() *BrowserTarget { return nil }

func (*ContentReadInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ContentReadInput) OperationName() string { return "content.read" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ContentReadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (cri *ContentReadInput) Timeout() time.Duration {
	ms := cri.DefaultTimeoutMS()
	if cri.TimeoutMS != nil {
		ms = *cri.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (cri *ContentReadInput) TabID() *TabID { return new(cri.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (cri *ContentReadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ContentReadInput) ElementTarget() *BrowserTarget { return nil }

func (*ContentFetchInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*ContentFetchInput) OperationName() string { return "content.fetch" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ContentFetchInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (cfi *ContentFetchInput) Timeout() time.Duration {
	ms := cfi.DefaultTimeoutMS()
	if cfi.TimeoutMS != nil {
		ms = *cfi.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (cfi *ContentFetchInput) TabID() *TabID { return nil }

// WaitURLPattern is the URL glob the action waits for after its input.
func (cfi *ContentFetchInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ContentFetchInput) ElementTarget() *BrowserTarget { return nil }

func (*AssetsListInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*AssetsListInput) OperationName() string { return "assets.list" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*AssetsListInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ali *AssetsListInput) Timeout() time.Duration {
	ms := ali.DefaultTimeoutMS()
	if ali.TimeoutMS != nil {
		ms = *ali.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ali *AssetsListInput) TabID() *TabID { return new(ali.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ali *AssetsListInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*AssetsListInput) ElementTarget() *BrowserTarget { return nil }

func (*AssetsExportInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*AssetsExportInput) OperationName() string { return "assets.export" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*AssetsExportInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (aei *AssetsExportInput) Timeout() time.Duration {
	ms := aei.DefaultTimeoutMS()
	if aei.TimeoutMS != nil {
		ms = *aei.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (aei *AssetsExportInput) TabID() *TabID { return new(aei.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (aei *AssetsExportInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*AssetsExportInput) ElementTarget() *BrowserTarget { return nil }

func (*CapabilitiesInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*CapabilitiesInput) OperationName() string { return "capabilities" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CapabilitiesInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (ci *CapabilitiesInput) Timeout() time.Duration {
	ms := ci.DefaultTimeoutMS()
	if ci.TimeoutMS != nil {
		ms = *ci.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (ci *CapabilitiesInput) TabID() *TabID { return new(ci.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (ci *CapabilitiesInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CapabilitiesInput) ElementTarget() *BrowserTarget { return nil }

func (*WebmcpListInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*WebmcpListInput) OperationName() string { return "webmcp.list" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*WebmcpListInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (wli *WebmcpListInput) Timeout() time.Duration {
	ms := wli.DefaultTimeoutMS()
	if wli.TimeoutMS != nil {
		ms = *wli.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (wli *WebmcpListInput) TabID() *TabID { return new(wli.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (wli *WebmcpListInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*WebmcpListInput) ElementTarget() *BrowserTarget { return nil }

func (*WebmcpCallInput) operation() {}

// OperationName returns the name without the browser. prefix.
func (*WebmcpCallInput) OperationName() string { return "webmcp.call" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*WebmcpCallInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (wci *WebmcpCallInput) Timeout() time.Duration {
	ms := wci.DefaultTimeoutMS()
	if wci.TimeoutMS != nil {
		ms = *wci.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (wci *WebmcpCallInput) TabID() *TabID { return new(wci.Tab) }

// WaitURLPattern is the URL glob the action waits for after its input.
func (wci *WebmcpCallInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*WebmcpCallInput) ElementTarget() *BrowserTarget { return nil }

// ParseQuery decodes the tree find --query reads from stdin and checks every base.
func ParseQuery(body []byte) (BrowserQuery, error) {
	q, err := DecodeBrowserQuery(body)
	if err != nil {
		return BrowserQuery{}, err
	}
	for branch := range q.branchRefs() {
		bases := 0
		if branch.Match != nil {
			bases++
		}
		if branch.And != nil {
			bases++
		}
		if branch.Or != nil {
			bases++
		}
		if bases != 1 {
			return BrowserQuery{}, errors.New("a query requires exactly one base: match, and, or")
		}
	}
	return q, nil
}

// Branches returns detached copies of this query and its nested queries in
// Rust's traversal order.
func (q BrowserQuery) Branches() []BrowserQuery {
	root := q.clone()
	var branches []BrowserQuery
	for branch := range root.branchRefs() {
		branches = append(branches, *branch)
	}
	return branches
}

// branchRefs visits a query tree internally without copying it for validation.
func (q *BrowserQuery) branchRefs() iter.Seq[*BrowserQuery] {
	return func(yield func(*BrowserQuery) bool) {
		pending := []*BrowserQuery{q}
		for len(pending) > 0 {
			branch := pending[len(pending)-1]
			pending = pending[:len(pending)-1]
			if !yield(branch) {
				return
			}
			for _, child := range []*BrowserQuery{branch.Within, branch.Frame, branch.Has, branch.HasNot} {
				if child != nil {
					pending = append(pending, child)
				}
			}
			for _, children := range []*[]BrowserQuery{branch.And, branch.Or} {
				if children != nil {
					for i := range *children {
						pending = append(pending, &(*children)[i])
					}
				}
			}
		}
	}
}

// Target converts a query's base locator to an unscoped element target.
func (q BrowserQueryMatch) Target() BrowserTarget { return BrowserTarget{BrowserQueryMatch: q.clone()} }
