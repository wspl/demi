package browserop

import (
	"errors"
	"fmt"
	"strings"
	"time"
)

// Operation is a checked invocation of the demi.browser package.
type Operation interface {
	operation()
	// OperationName is the full operation name in the package descriptor.
	OperationName() string
}

// Input answers the scheduling and targeting questions shared by browser commands.
type Input interface {
	Operation
	Timeout() time.Duration
	DefaultTimeoutMS() uint64
	TabID() *TabID
	ElementTarget() *BrowserTarget
	WaitURLPattern() *string
}

// UnknownOperation is a name outside this package.
type UnknownOperation struct{ Name string }

func (e *UnknownOperation) Error() string { return fmt.Sprintf("unknown operation %s", e.Name) }

// UnservedOperation is a browser operation this package does not serve.
type UnservedOperation struct{ Name string }

func (e *UnservedOperation) Error() string { return fmt.Sprintf("unknown operation %s", e.Name) }

// InvalidInput wraps the reason an operation's arguments were refused.
type InvalidInput struct{ Err error }

func (e *InvalidInput) Error() string { return e.Err.Error() }
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
	case "close":
		v, err := DecodeCloseInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
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
	case "wait":
		v, err := DecodeWaitInput(args)
		if err != nil {
			return nil, &InvalidInput{Err: err}
		}
		return &v, nil
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

// OperationName returns the full name in the package descriptor.
func (*LiveInput) OperationName() string { return LiveOperation }

func (*OpenInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*OpenInput) OperationName() string { return "browser.open" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*OpenInput) DefaultTimeoutMS() uint64 { return MaxTimeoutMS }

// Timeout is the whole operation's deadline.
func (v *OpenInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *OpenInput) TabID() *TabID { return nil }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *OpenInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*OpenInput) ElementTarget() *BrowserTarget { return nil }

func (*TabsInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*TabsInput) OperationName() string { return "browser.tabs" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*TabsInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *TabsInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *TabsInput) TabID() *TabID { return nil }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *TabsInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*TabsInput) ElementTarget() *BrowserTarget { return nil }

func (*InfoInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*InfoInput) OperationName() string { return "browser.info" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*InfoInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *InfoInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *InfoInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *InfoInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*InfoInput) ElementTarget() *BrowserTarget { return nil }

func (*GotoInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*GotoInput) OperationName() string { return "browser.goto" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*GotoInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *GotoInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *GotoInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *GotoInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*GotoInput) ElementTarget() *BrowserTarget { return nil }

func (*BackInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*BackInput) OperationName() string { return "browser.back" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*BackInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *BackInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *BackInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *BackInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*BackInput) ElementTarget() *BrowserTarget { return nil }

func (*ForwardInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ForwardInput) OperationName() string { return "browser.forward" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ForwardInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ForwardInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ForwardInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ForwardInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ForwardInput) ElementTarget() *BrowserTarget { return nil }

func (*ReloadInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ReloadInput) OperationName() string { return "browser.reload" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ReloadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ReloadInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ReloadInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ReloadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ReloadInput) ElementTarget() *BrowserTarget { return nil }

func (*HistoryInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*HistoryInput) OperationName() string { return "browser.history" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*HistoryInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *HistoryInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *HistoryInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *HistoryInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*HistoryInput) ElementTarget() *BrowserTarget { return nil }

func (*CloseInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*CloseInput) OperationName() string { return "browser.close" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CloseInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *CloseInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *CloseInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *CloseInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CloseInput) ElementTarget() *BrowserTarget { return nil }

func (*InspectInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*InspectInput) OperationName() string { return "browser.inspect" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*InspectInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *InspectInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *InspectInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *InspectInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*InspectInput) ElementTarget() *BrowserTarget { return nil }

func (*FindInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*FindInput) OperationName() string { return "browser.find" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*FindInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *FindInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *FindInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *FindInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *FindInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*ReadInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ReadInput) OperationName() string { return "browser.read" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ReadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ReadInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ReadInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ReadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *ReadInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*ScreenshotInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ScreenshotInput) OperationName() string { return "browser.screenshot" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ScreenshotInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ScreenshotInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ScreenshotInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ScreenshotInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ScreenshotInput) ElementTarget() *BrowserTarget { return nil }

func (*ProbeInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ProbeInput) OperationName() string { return "browser.probe" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ProbeInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ProbeInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ProbeInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ProbeInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ProbeInput) ElementTarget() *BrowserTarget { return nil }

func (*ClickInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ClickInput) OperationName() string { return "browser.click" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ClickInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ClickInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ClickInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ClickInput) WaitURLPattern() *string { return v.WaitURL }

// ElementTarget is the element target the input carries.
func (v *ClickInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*MoveInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*MoveInput) OperationName() string { return "browser.move" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*MoveInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *MoveInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *MoveInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *MoveInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *MoveInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*DragInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*DragInput) OperationName() string { return "browser.drag" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DragInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *DragInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *DragInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *DragInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*DragInput) ElementTarget() *BrowserTarget { return nil }

func (*ScrollInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ScrollInput) OperationName() string { return "browser.scroll" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ScrollInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ScrollInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ScrollInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ScrollInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *ScrollInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*FillInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*FillInput) OperationName() string { return "browser.fill" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*FillInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *FillInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *FillInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *FillInput) WaitURLPattern() *string { return v.WaitURL }

// ElementTarget is the element target the input carries.
func (v *FillInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*TypeInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*TypeInput) OperationName() string { return "browser.type" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*TypeInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *TypeInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *TypeInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *TypeInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *TypeInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*KeyInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*KeyInput) OperationName() string { return "browser.key" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*KeyInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *KeyInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *KeyInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *KeyInput) WaitURLPattern() *string { return v.WaitURL }

// ElementTarget is the element target the input carries.
func (v *KeyInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*CheckInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*CheckInput) OperationName() string { return "browser.check" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CheckInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *CheckInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *CheckInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *CheckInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *CheckInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*SelectInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*SelectInput) OperationName() string { return "browser.select" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*SelectInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *SelectInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *SelectInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *SelectInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *SelectInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*SelectTextInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*SelectTextInput) OperationName() string { return "browser.select-text" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*SelectTextInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *SelectTextInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *SelectTextInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *SelectTextInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *SelectTextInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*WaitInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*WaitInput) OperationName() string { return "browser.wait" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*WaitInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *WaitInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *WaitInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *WaitInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *WaitInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*UploadInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*UploadInput) OperationName() string { return "browser.upload" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*UploadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *UploadInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *UploadInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *UploadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *UploadInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*DownloadInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*DownloadInput) OperationName() string { return "browser.download" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DownloadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *DownloadInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *DownloadInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *DownloadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *DownloadInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*ClipboardWriteInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ClipboardWriteInput) OperationName() string { return "browser.clipboard.write" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ClipboardWriteInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ClipboardWriteInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ClipboardWriteInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ClipboardWriteInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ClipboardWriteInput) ElementTarget() *BrowserTarget { return nil }

func (*ClipboardReadInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ClipboardReadInput) OperationName() string { return "browser.clipboard.read" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ClipboardReadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ClipboardReadInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ClipboardReadInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ClipboardReadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ClipboardReadInput) ElementTarget() *BrowserTarget { return nil }

func (*EvalInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*EvalInput) OperationName() string { return "browser.eval" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*EvalInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *EvalInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *EvalInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *EvalInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (v *EvalInput) ElementTarget() *BrowserTarget {
	return &v.BrowserTarget
}

func (*LogsInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*LogsInput) OperationName() string { return "browser.logs" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*LogsInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *LogsInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *LogsInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *LogsInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*LogsInput) ElementTarget() *BrowserTarget { return nil }

func (*ViewportSetInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ViewportSetInput) OperationName() string { return "browser.viewport.set" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ViewportSetInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ViewportSetInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ViewportSetInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ViewportSetInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ViewportSetInput) ElementTarget() *BrowserTarget { return nil }

func (*ViewportResetInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ViewportResetInput) OperationName() string { return "browser.viewport.reset" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ViewportResetInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ViewportResetInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ViewportResetInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ViewportResetInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ViewportResetInput) ElementTarget() *BrowserTarget { return nil }

func (*DialogInspectInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*DialogInspectInput) OperationName() string { return "browser.dialog.inspect" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DialogInspectInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *DialogInspectInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *DialogInspectInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *DialogInspectInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*DialogInspectInput) ElementTarget() *BrowserTarget { return nil }

func (*DialogAcceptInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*DialogAcceptInput) OperationName() string { return "browser.dialog.accept" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DialogAcceptInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *DialogAcceptInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *DialogAcceptInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *DialogAcceptInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*DialogAcceptInput) ElementTarget() *BrowserTarget { return nil }

func (*DialogDismissInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*DialogDismissInput) OperationName() string { return "browser.dialog.dismiss" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*DialogDismissInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *DialogDismissInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *DialogDismissInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *DialogDismissInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*DialogDismissInput) ElementTarget() *BrowserTarget { return nil }

func (*CdpTargetsInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*CdpTargetsInput) OperationName() string { return "browser.cdp.targets" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CdpTargetsInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *CdpTargetsInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *CdpTargetsInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *CdpTargetsInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CdpTargetsInput) ElementTarget() *BrowserTarget { return nil }

func (*CdpDetachInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*CdpDetachInput) OperationName() string { return "browser.cdp.detach" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CdpDetachInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *CdpDetachInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *CdpDetachInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *CdpDetachInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CdpDetachInput) ElementTarget() *BrowserTarget { return nil }

func (*CdpSendInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*CdpSendInput) OperationName() string { return "browser.cdp.send" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CdpSendInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *CdpSendInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *CdpSendInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *CdpSendInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CdpSendInput) ElementTarget() *BrowserTarget { return nil }

func (*CdpEventsInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*CdpEventsInput) OperationName() string { return "browser.cdp.events" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CdpEventsInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *CdpEventsInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *CdpEventsInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *CdpEventsInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CdpEventsInput) ElementTarget() *BrowserTarget { return nil }

func (*ContentReadInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ContentReadInput) OperationName() string { return "browser.content.read" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ContentReadInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ContentReadInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ContentReadInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ContentReadInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ContentReadInput) ElementTarget() *BrowserTarget { return nil }

func (*ContentFetchInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*ContentFetchInput) OperationName() string { return "browser.content.fetch" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*ContentFetchInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *ContentFetchInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *ContentFetchInput) TabID() *TabID { return nil }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *ContentFetchInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*ContentFetchInput) ElementTarget() *BrowserTarget { return nil }

func (*AssetsListInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*AssetsListInput) OperationName() string { return "browser.assets.list" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*AssetsListInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *AssetsListInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *AssetsListInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *AssetsListInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*AssetsListInput) ElementTarget() *BrowserTarget { return nil }

func (*AssetsExportInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*AssetsExportInput) OperationName() string { return "browser.assets.export" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*AssetsExportInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *AssetsExportInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *AssetsExportInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *AssetsExportInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*AssetsExportInput) ElementTarget() *BrowserTarget { return nil }

func (*CapabilitiesInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*CapabilitiesInput) OperationName() string { return "browser.capabilities" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*CapabilitiesInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *CapabilitiesInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *CapabilitiesInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *CapabilitiesInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*CapabilitiesInput) ElementTarget() *BrowserTarget { return nil }

func (*WebmcpListInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*WebmcpListInput) OperationName() string { return "browser.webmcp.list" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*WebmcpListInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *WebmcpListInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *WebmcpListInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *WebmcpListInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*WebmcpListInput) ElementTarget() *BrowserTarget { return nil }

func (*WebmcpCallInput) operation() {}

// OperationName returns the full name in the package descriptor.
func (*WebmcpCallInput) OperationName() string { return "browser.webmcp.call" }

// DefaultTimeoutMS is the deadline when the input names none, in milliseconds.
func (*WebmcpCallInput) DefaultTimeoutMS() uint64 { return TimeoutMS }

// Timeout is the whole operation's deadline.
func (v *WebmcpCallInput) Timeout() time.Duration {
	ms := v.DefaultTimeoutMS()
	if v.TimeoutMS != nil {
		ms = *v.TimeoutMS
	}
	return time.Duration(ms) * time.Millisecond
}

// TabID is the tab this operation acts on, or nil for an untargeted command.
func (v *WebmcpCallInput) TabID() *TabID { return &v.Tab }

// WaitURLPattern is the URL glob the action waits for after its input.
func (v *WebmcpCallInput) WaitURLPattern() *string { return nil }

// ElementTarget is the element target the input carries.
func (*WebmcpCallInput) ElementTarget() *BrowserTarget { return nil }

// ParseQuery decodes the tree find --query reads from stdin and checks every base.
func ParseQuery(body []byte) (BrowserQuery, error) {
	q, err := DecodeBrowserQuery(body)
	if err != nil {
		return BrowserQuery{}, err
	}
	for _, branch := range q.Branches() {
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

// Branches returns this query and every nested query in Rust's traversal order.
func (q *BrowserQuery) Branches() []*BrowserQuery {
	pending := []*BrowserQuery{q}
	var branches []*BrowserQuery
	for len(pending) > 0 {
		branch := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		branches = append(branches, branch)
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
	return branches
}

// Target converts a query's base locator to an unscoped element target.
func (q BrowserQueryMatch) Target() BrowserTarget { return BrowserTarget{BrowserQueryMatch: q} }
