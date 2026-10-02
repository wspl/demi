package page

import (
	"context"
	"errors"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/chromedp/cdproto/browser"
	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/dom"
	"github.com/chromedp/cdproto/input"
	"github.com/chromedp/cdproto/network"
	chrome "github.com/chromedp/cdproto/page"
	jsonv2 "github.com/go-json-experiment/json"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

type downloadPhase uint8

const (
	downloadNotStarted downloadPhase = iota
	downloadTriggered
	downloadActive
	downloadFinished
)

// Download observes and saves the download triggered by one click.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func Download(ctx context.Context, invocation *cmdsdk.InvocationContext[commandwire.Invocation], environment *tabs.Environment, tab *tabs.Tab, request browserop.DownloadInput, deadline time.Time) (result browserop.DownloadResult, err error) {
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
	beginnings, err := environment.Subscribe("Browser.downloadWillBegin")
	if err != nil {
		return result, err
	}
	defer beginnings.Close()
	progress, err := environment.Subscribe("Browser.downloadProgress")
	if err != nil {
		return result, err
	}
	defer progress.Close()
	responses, err := tab.Subscribe("Network.responseReceived")
	if err != nil {
		return result, err
	}
	defer responses.Close()
	snapshot, err := cdp.CaptureFrames(operation.Context(), tab.Page())
	if err != nil {
		return result, err
	}
	phase := downloadNotStarted
	var download *browser.EventDownloadWillBegin
	err = operation.Run(ctx, func(work context.Context) error {
		var output string
		var err error
		overwrite := browserOption(request.Overwrite, false)
		if request.Output != nil {
			output, err = cdp.Preflight(work, invocation.Request.Cwd, *request.Output, overwrite)
		} else {
			output, err = environment.SavedDownload()
		}
		if err != nil {
			return err
		}
		var p point
		if request.XY != nil {
			p, err = coordinates(work, tab, *request.XY)
		} else {
			var last error
			ready, readyErr := ready(work, tab, request.BrowserTarget, &checkout.Session().References, []string{"visible", "enabled", "geometry", "stable", "hit"}, operation, &last)
			err = readyErr
			p = point{ready.state.X, ready.state.Y}
		}
		if err != nil {
			return err
		}
		if request.XY != nil {
			media, mediaErr := downloadMedia(work, tab, snapshot, p, invocation.Request.Cwd, output, overwrite)
			if mediaErr != nil {
				return mediaErr
			}
			if media != nil {
				result = *media
				return nil
			}
		}
		phase = downloadTriggered
		if err = clickAt(work, tab, p, input.Left, 1, 0, operation); err != nil {
			return err
		}
		download, err = nextDownload(work, beginnings, snapshot)
		if err != nil {
			return err
		}
		phase = downloadActive
		state, err := downloadProgress(work, progress, download.GUID)
		if err != nil {
			return err
		}
		phase = downloadFinished
		if state == browser.DownloadProgressStateCanceled {
			return &cdp.BrowserError{Kind: cdp.KindIO, Message: "Chrome cancelled the download"}
		}
		source, err := spoolPath(environment, download.GUID)
		if err != nil {
			return err
		}
		info, err := os.Stat(source)
		if err != nil {
			return &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
		}
		mime := "application/octet-stream"
		// Next drains queued events before honoring cancellation, so this context
		// implements Rust's now_or_never without a timer or a background reader.
		drain, cancel := context.WithCancel(work)
		cancel()
		for {
			event, err := responses.Next(drain)
			if errors.Is(err, context.Canceled) {
				break
			}
			if err != nil {
				return err
			}
			var response network.EventResponseReceived
			if err = jsonv2.Unmarshal(event.Params, &response); err != nil {
				return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
			}
			if response.Response.URL == download.URL {
				mime = response.Response.MimeType
			}
		}
		path, err := cdp.PublishFile(work, invocation.Request.Cwd, output, source, overwrite)
		if err != nil {
			return err
		}
		result = browserop.DownloadResult{Path: path, SuggestedFilename: download.SuggestedFilename, Bytes: uint64(info.Size()), MIMEType: mime}
		return nil
	})
	if err != nil && phase == downloadTriggered {
		cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
		found, findErr := nextDownload(cleanup, beginnings, snapshot)
		cancel()
		if findErr == nil {
			download = found
			phase = downloadActive
		} else if !errors.Is(findErr, context.DeadlineExceeded) {
			err = cdp.AfterCleanup(err, findErr)
		}
	}
	if phase == downloadActive || phase == downloadFinished {
		source, pathErr := spoolPath(environment, download.GUID)
		cleanupErr := pathErr
		if pathErr == nil {
			if phase == downloadActive {
				cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
				stopErr := browser.CancelDownload(download.GUID).Do(protocol.WithExecutor(cleanup, environment.Browser()))
				if stopErr == nil {
					_, stopErr = downloadProgress(cleanup, progress, download.GUID)
				}
				cancel()
				cleanupErr = cdp.AfterCleanup(cleanupErr, stopErr)
			}
			for _, path := range []string{source, filepath.Join(environment.DownloadDirectory(), download.GUID+".crdownload")} {
				removeErr := os.Remove(path)
				if errors.Is(removeErr, os.ErrNotExist) {
					removeErr = nil
				} // Chrome may remove cancelled partials.
				if removeErr != nil {
					removeErr = &cdp.BrowserError{Kind: cdp.KindIO, Cause: removeErr}
				}
				cleanupErr = cdp.AfterCleanup(cleanupErr, removeErr)
			}
		}
		err = cdp.AfterCleanup(err, cleanupErr)
	}
	err = cdp.AfterCleanup(err, releaseObjects(ctx, tab))
	if err != nil {
		return result, operation.Failure(err, string(tab.ID()), nil)
	}
	return result, nil
}

// nextDownload ignores events from frames outside this invocation's tab.
func nextDownload(ctx context.Context, events *cdp.Subscription, snapshot cdp.FrameSnapshot) (*browser.EventDownloadWillBegin, error) {
	for {
		event, err := events.Next(ctx)
		if err != nil {
			return nil, err
		}
		var begin browser.EventDownloadWillBegin
		if err = jsonv2.Unmarshal(event.Params, &begin); err != nil {
			return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
		}
		for _, document := range snapshot.Frames {
			if document.Frame.ID == begin.FrameID {
				return &begin, nil
			}
		}
	}
}

// downloadProgress waits for the observed GUID to finish or be cancelled.
func downloadProgress(ctx context.Context, events *cdp.Subscription, guid string) (browser.DownloadProgressState, error) {
	for {
		event, err := events.Next(ctx)
		if err != nil {
			return "", err
		}
		var progress browser.EventDownloadProgress
		if err = jsonv2.Unmarshal(event.Params, &progress); err != nil {
			return "", &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
		}
		if progress.GUID == guid && progress.State != browser.DownloadProgressStateInProgress {
			return progress.State, nil
		}
	}
}

// spoolPath confines Chrome's download identifier to the private download spool.
func spoolPath(environment *tabs.Environment, guid string) (string, error) {
	if guid == "" || strings.IndexFunc(guid, func(r rune) bool {
		return (r < 'a' || r > 'z') && (r < 'A' || r > 'Z') && (r < '0' || r > '9') && r != '-'
	}) >= 0 {
		return "", &cdp.BrowserError{Kind: cdp.KindCDP, Message: "download identifier is not a safe spool name"}
	}
	return filepath.Join(environment.DownloadDirectory(), guid), nil
}

// downloadMedia saves the resource of an image, video or audio hit at coordinates.
func downloadMedia(ctx context.Context, tab *tabs.Tab, snapshot cdp.FrameSnapshot, p point, cwd, output string, overwrite bool) (*browserop.DownloadResult, error) {
	backend, frame, _, err := dom.GetNodeForLocation(int64(p.x), int64(p.y)).Do(protocol.WithExecutor(ctx, tab.Page()))
	if err != nil {
		return nil, err
	}
	var renderer cdp.FrameTarget
	for _, document := range snapshot.Frames {
		if document.Frame.ID == frame {
			renderer = document.Target
			break
		}
	}
	if renderer == nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindTargetNotFound}
	}
	media, err := resolveElement(ctx, renderer, backend)
	if err != nil {
		return nil, err
	}
	source, err := decodeElement[*string](ctx, media, "function() { return ['img','video','audio'].includes(this.localName) ? (this.currentSrc || this.src || null) : null; }", false)
	if err != nil || source == nil {
		return nil, err
	}
	s, err := preparedState(ctx, media, []string{"visible", "enabled", "geometry", "stable", "hit"}, true)
	if err != nil {
		return nil, err
	}
	if s.Failed != nil {
		return nil, s.failure()
	}
	tree, err := chrome.GetResourceTree().Do(protocol.WithExecutor(ctx, renderer))
	if err != nil {
		return nil, err
	}
	mime := "application/octet-stream"
	pending := []*chrome.FrameResourceTree{tree}
	for len(pending) > 0 {
		item := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		pending = append(pending, item.ChildFrames...)
		if item.Frame.ID == frame {
			for _, resource := range item.Resources {
				if resource.URL == *source {
					mime = resource.MimeType
					break
				}
			}
		}
	}
	data, err := resourceBytes(ctx, renderer, frame, *source)
	if err != nil {
		return nil, err
	}
	suggested := ""
	if parsed, parseErr := url.Parse(*source); parseErr == nil {
		parts := strings.Split(parsed.EscapedPath(), "/")
		suggested = parts[len(parts)-1]
	}
	path, err := cdp.SaveWithOverwrite(ctx, cwd, output, data, overwrite)
	if err != nil {
		return nil, err
	}
	return &browserop.DownloadResult{Path: path, SuggestedFilename: suggested, Bytes: uint64(len(data)), MIMEType: mime}, nil
}
