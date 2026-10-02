package page

import (
	"bytes"
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"image/png"
	"io"
	"os"
	"path/filepath"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
)

// ClipboardWrite replaces the isolated browser clipboard with bounded invocation input.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func ClipboardWrite(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], environment *tabs.Environment, tab *tabs.Tab, input browserop.ClipboardWriteInput, deadline time.Time) (result browserop.ClipboardWriteResult, err error) {
	if tab == nil {
		return result, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	err = operation.Run(ctx, func(work context.Context) error {
		capability, err := ClipboardCapability(work, tab)
		if err != nil {
			return err
		}
		if !capability.Available {
			return &cdp.BrowserError{Kind: cdp.KindUnsupportedCapability, Message: browserOption(capability.Reason, "clipboard unavailable")}
		}
		checkout := tab.Gate().TryCheckout()
		if checkout == nil {
			return &cdp.BrowserError{Kind: cdp.KindBusy}
		}
		defer checkout.Release()
		if err = GrantClipboard(work, environment.Browser()); err != nil {
			return err
		}
		mime := browserOption(input.MIME, browserop.ClipboardMime("text/plain"))
		maximum := browserop.StdinBytes
		if mime == "image/png" {
			maximum = browserop.ClipboardPNGBytes
		}
		var data []byte
		for {
			chunk, err := invocation.Input.Next(work)
			if errors.Is(err, io.EOF) {
				break
			}
			if err != nil {
				return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: err.Error(), Cause: err}
			}
			if len(data)+len(chunk) > maximum {
				return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: fmt.Sprintf("%s clipboard input exceeds %d bytes", mime, maximum)}
			}
			data = append(data, chunk...)
		}
		if err = validateClipboard(mime, data); err != nil {
			return err
		}
		encoded, err := cdp.Value(base64.StdEncoding.EncodeToString(data))
		if err != nil {
			return err
		}
		mimeJSON, err := cdp.Value(mime)
		if err != nil {
			return err
		}
		script := fmt.Sprintf(`(async () => {
 const bytes = Uint8Array.from(atob(%s), value => value.charCodeAt(0));
 await navigator.clipboard.write([new ClipboardItem({[%s]: new Blob([bytes], {type:%s})})]);
 return true;
 })()`, encoded, mimeJSON, mimeJSON)
		operation.BeginInput()
		written, err := evaluateValue[bool](work, tab.Page(), script)
		if err != nil {
			return err
		}
		if !written {
			return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "clipboard write did not complete"}
		}
		operation.CompleteInput()
		result = browserop.ClipboardWriteResult{MIMEType: mime, Bytes: uint(len(data))}
		return nil
	})
	if err != nil {
		return result, operation.Failure(err, string(tab.ID()), nil)
	}
	return result, nil
}

// ClipboardRead reads the isolated browser clipboard.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func ClipboardRead(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], environment *tabs.Environment, tab *tabs.Tab, input browserop.ClipboardReadInput, deadline time.Time) (result browserop.ClipboardReadResult, err error) {
	if tab == nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	err = operation.Run(ctx, func(work context.Context) error {
		capability, err := ClipboardCapability(work, tab)
		if err != nil {
			return err
		}
		if !capability.Available {
			return &cdp.BrowserError{Kind: cdp.KindUnsupportedCapability, Message: browserOption(capability.Reason, "clipboard unavailable")}
		}
		checkout := tab.Gate().TryCheckout()
		if checkout == nil {
			return &cdp.BrowserError{Kind: cdp.KindBusy}
		}
		defer checkout.Release()
		if err = GrantClipboard(work, environment.Browser()); err != nil {
			return err
		}
		if (input.Format != nil) == (input.OutputDir != nil) {
			return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "clipboard read requires --format text or --output-dir"}
		}
		if input.Format != nil {
			text, err := evaluateValue[string](work, tab.Page(), "navigator.clipboard.readText()")
			if err != nil {
				return err
			}
			if len(text) > browserop.StdinBytes {
				return &cdp.BrowserError{Kind: cdp.KindResultTooLarge}
			}
			result = &browserop.ClipboardReadResultText{Text: text}
			return nil
		}
		script := fmt.Sprintf(`(async () => {
 const result = [];
 let total = 0;
 for (const item of await navigator.clipboard.read()) {
  for (const mimeType of item.types) {
   if (!['text/plain','text/html','image/png'].includes(mimeType)) continue;
   const blob = await item.getType(mimeType);
   const limit = mimeType === 'image/png' ? %d : %d;
   total += blob.size;
   if (blob.size > limit || total > %d) throw new Error('clipboard result exceeds the byte limit');
   const bytes = new Uint8Array(await blob.arrayBuffer());
   let text = '';
   for (let offset = 0; offset < bytes.length; offset += 32768) text += String.fromCharCode(...bytes.subarray(offset, offset + 32768));
   result.push({mimeType, data:btoa(text)});
  }
 }
 return result;
 })()`, browserop.ClipboardPNGBytes, browserop.StdinBytes, browserop.ClipboardPNGBytes)
		type entry struct {
			MIME browserop.ClipboardMime `json:"mimeType"`
			Data string                  `json:"data"`
		}
		items, err := evaluateValue[[]entry](work, tab.Page(), script)
		if err != nil {
			return err
		}
		directory, err := cdp.Resolve(invocation.Request.Cwd, *input.OutputDir)
		if err != nil {
			return err
		}
		if err = os.MkdirAll(directory, 0755); err != nil {
			return &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
		}
		type decoded struct {
			mime browserop.ClipboardMime
			path string
			data []byte
		}
		decodedItems := make([]decoded, 0, len(items))
		for i, item := range items {
			data, err := base64.StdEncoding.DecodeString(item.Data)
			if err != nil {
				return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
			}
			if err = validateClipboard(item.MIME, data); err != nil {
				return err
			}
			extension := "txt"
			switch item.MIME {
			case "text/html":
				extension = "html"
			case "image/png":
				extension = "png"
			}
			path := filepath.Join(directory, fmt.Sprintf("item-%d.%s", i, extension))
			if _, err = cdp.Preflight(work, invocation.Request.Cwd, path, browserOption(input.Overwrite, false)); err != nil {
				return err
			}
			decodedItems = append(decodedItems, decoded{item.MIME, path, data})
		}
		saved := []browserop.ClipboardItem{}
		for _, item := range decodedItems {
			path, err := cdp.SaveWithOverwrite(work, invocation.Request.Cwd, item.path, item.data, browserOption(input.Overwrite, false))
			if err != nil {
				return err
			}
			saved = append(saved, browserop.ClipboardItem{MIMEType: item.mime, Path: path, Bytes: uint(len(item.data))})
		}
		result = &browserop.ClipboardReadResultItems{Items: saved}
		return nil
	})
	return result, err
}

// evaluateValue decodes the result of a browser-owned promise at the CDP boundary.
func evaluateValue[T any](ctx context.Context, executor cdp.Executor, script string) (T, error) {
	var result T
	object, exception, err := runtime.Evaluate(script).WithAwaitPromise(true).WithReturnByValue(true).Do(protocol.WithExecutor(ctx, executor))
	if err != nil {
		return result, err
	}
	if err = evaluationException(exception); err != nil {
		return result, err
	}
	if object == nil {
		return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "evaluation returned no JSON value"}
	}
	return decodePageValue[T](object.Value)
}

// validateClipboard validates the bytes before replacing or exporting clipboard data.
func validateClipboard(mime browserop.ClipboardMime, data []byte) error {
	if err := mime.Validate(); err != nil {
		return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	maximum := browserop.StdinBytes
	if mime == "image/png" {
		maximum = browserop.ClipboardPNGBytes
	}
	if len(data) > maximum {
		return &cdp.BrowserError{Kind: cdp.KindResultTooLarge}
	}
	if mime != "image/png" {
		if err := contract.CheckUTF8(data); err != nil {
			return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "clipboard text is not UTF-8: " + err.Error(), Cause: err}
		}
		return nil
	}
	config, err := png.DecodeConfig(bytes.NewReader(data))
	if err != nil {
		return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "invalid clipboard PNG: " + err.Error(), Cause: err}
	}
	if uint64(config.Width)*uint64(config.Height) > browserop.ClipboardPNGPixels {
		return &cdp.BrowserError{Kind: cdp.KindResultTooLarge}
	}
	if _, err = png.Decode(bytes.NewReader(data)); err != nil {
		return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "invalid clipboard PNG: " + err.Error(), Cause: err}
	}
	return nil
}
