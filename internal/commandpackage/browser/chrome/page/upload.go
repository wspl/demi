package page

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"time"
	"unicode/utf8"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/dom"
	chrome "github.com/chromedp/cdproto/page"
	jsonv2 "github.com/go-json-experiment/json"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
)

// Upload attaches validated Host files to a file input.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func Upload(
	ctx context.Context,
	invocation *commandsdk.InvocationContext[commandproto.Invocation],
	tab *tabs.Tab,
	input browserproto.UploadInput,
	deadline time.Time,
) (browserproto.UploadResult, error) {
	result := browserproto.UploadResult{Files: []string{}}
	if tab == nil {
		return result, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return result, operation.Failure(&cdp.BrowserError{Kind: cdp.KindBusy}, string(tab.ID()), nil)
	}
	defer checkout.Release()
	err := operation.Run(ctx, func(context.Context) error {
		for _, file := range input.File {
			path, err := uploadPath(invocation.Request.Cwd, file)
			if err != nil {
				return err
			}
			result.Files = append(result.Files, path)
		}
		return nil
	})
	if err == nil {
		var last error
		var control readyElement
		control, err = ready(
			ctx,
			tab,
			input.BrowserTarget,
			&checkout.Session().References,
			[]string{"enabled"},
			operation,
			&last,
		)
		if err == nil {
			err = operation.Run(ctx, func(work context.Context) error {
				return uploadToControl(work, tab, control, result, operation)
			})
		}
	}
	err = cdp.AfterCleanup(err, releaseObjects(ctx, tab))
	if err != nil {
		return result, operation.Failure(err, string(tab.ID()), nil)
	}
	result.Attached = uint(len(result.Files))
	return result, nil
}

// attachFiles checks multiplicity, attaches files and verifies the retained list.
func attachFiles(ctx context.Context, element targetElement, files []string, operation *cdp.Operation) error {
	s, err := state(ctx, element, []string{"enabled"}, false)
	if err != nil {
		return err
	}
	if s.Failed != nil {
		return s.failure()
	}
	multiple, err := decodeElement[bool](ctx, element, "function() { return this.multiple === true; }", false)
	if err != nil {
		return err
	}
	if len(files) > 1 && !multiple {
		return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "multiple files require a multiple file input"}
	}
	operation.BeginInput()
	if err = dom.SetFileInputFiles(files).
		WithBackendNodeID(element.backend).
		Do(protocol.WithExecutor(ctx, element.page)); err != nil {
		return err
	}
	operation.CompleteInput()
	attached, err := decodeElement[uint](ctx, element, "function() { return this.files.length; }", false)
	if err != nil {
		return err
	}
	if attached != uint(len(files)) {
		condition := "the page did not retain the attached files"
		return &cdp.BrowserError{Kind: cdp.KindNotActionable, Details: browserproto.ErrorDetails{Condition: &condition}}
	}
	return nil
}

type subscriber interface {
	Subscribe(...string) (*cdp.Subscription, error)
}

// chooserUpload subscribes before triggering and always disables chooser interception.
func chooserUpload(
	ctx context.Context,
	tab *tabs.Tab,
	control targetElement,
	files []string,
	operation *cdp.Operation,
) (err error) {
	source, ok := control.page.(subscriber)
	if !ok {
		return &cdp.BrowserError{
			Kind:    cdp.KindUnsupportedCapability,
			Message: "renderer does not expose event subscriptions",
		}
	}
	choosers, err := source.Subscribe("Page.fileChooserOpened")
	if err != nil {
		return err
	}
	defer choosers.Close()
	defer func() {
		cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
		defer cancel()
		releaseErr := chrome.SetInterceptFileChooserDialog(false).Do(protocol.WithExecutor(cleanup, control.page))
		var failure *cdp.BrowserError
		if errors.As(releaseErr, &failure) &&
			(failure.Kind == cdp.KindClosed || failure.Kind == cdp.KindConnection || failure.Kind == cdp.KindTabNotFound) {
			releaseErr = nil
		}
		err = cdp.AfterCleanup(err, releaseErr)
	}()
	if err = chrome.SetInterceptFileChooserDialog(true).Do(protocol.WithExecutor(ctx, control.page)); err != nil {
		return err
	}
	err = tab.Input(ctx, operation, func(work context.Context) error {
		operation.BeginInput()
		_, err := elementCall(work, control, "function() { this.click(); return null; }", true)
		if err == nil {
			operation.CompleteInput()
		}
		return err
	})
	if err != nil {
		return err
	}
	return attachChosenFiles(ctx, choosers, control, files, operation)
}

func uploadPath(cwd string, file browserproto.LocatorText) (string, error) {
	path, err := cdp.Resolve(cwd, string(file))
	if err != nil {
		return "", err
	}
	path, err = filepath.EvalSymlinks(path)
	if err != nil {
		return "", &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
	}
	info, err := os.Stat(path)
	if err != nil {
		return "", &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
	}
	if !info.Mode().IsRegular() {
		return "", &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "upload paths must name readable regular files",
		}
	}
	opened, err := os.Open(path)
	if err != nil {
		return "", &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
	}
	if err = opened.Close(); err != nil {
		return "", &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
	}
	if !utf8.ValidString(path) {
		return "", &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "upload paths must be UTF-8"}
	}

	return path, nil
}

func attachChosenFiles(
	ctx context.Context,
	choosers *cdp.Subscription,
	control targetElement,
	files []string,
	operation *cdp.Operation,
) error {
	event, err := choosers.Next(ctx)
	if err != nil {
		return err
	}
	var chooser chrome.EventFileChooserOpened
	if err = jsonv2.Unmarshal(event.Params, &chooser); err != nil {
		return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	if len(files) > 1 && chooser.Mode != chrome.FileChooserOpenedModeSelectMultiple {
		return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "multiple files require a multiple file input"}
	}
	if chooser.BackendNodeID == 0 {
		return &cdp.BrowserError{
			Kind:    cdp.KindUnsupportedCapability,
			Message: "the chooser does not expose an attachable file input",
		}
	}
	element, err := resolveElement(ctx, control.page, chooser.BackendNodeID)
	if err != nil {
		return err
	}
	return attachFiles(ctx, element, files, operation)
}

func uploadToControl(
	work context.Context,
	tab *tabs.Tab,
	control readyElement,
	result browserproto.UploadResult,
	operation *cdp.Operation,
) error {
	isInput, err := decodeElement[bool](
		work,
		control.element,
		"function() { return this.localName === 'input' && this.type === 'file'; }",
		false,
	)
	if err != nil {
		return err
	}
	if isInput {
		return attachFiles(work, control.element, result.Files, operation)
	}
	return chooserUpload(work, tab, control.element, result.Files, operation)
}
