package page

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// ContentFetch reads URLs through environment-owned temporary tabs.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func ContentFetch(
	ctx context.Context,
	invocation *cmdsdk.InvocationContext[commandwire.Invocation],
	environment *tabs.Environment,
	input browserop.ContentFetchInput,
	deadline time.Time,
) (result browserop.ContentFetchResult, err error) {
	result.Pages = []browserop.FetchedPage{}
	for _, url := range input.URL {
		if err = tabs.ValidateURL(string(url)); err != nil {
			return result, err
		}
	}
	caller, err := cdp.Agent(invocation)
	if err != nil {
		return result, err
	}
	batch, err := environment.TemporaryTabs(ctx, caller, uint(len(input.URL)), deadline)
	if err != nil {
		return result, err
	}
	defer func() {
		cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
		defer cancel()
		err = cdp.AfterCleanup(err, batch.Close(cleanup, cdp.ControlTimeout))
	}()
	for i, tab := range batch.Tabs() {
		requested := string(input.URL[i])
		page, itemErr := fetchPage(
			ctx,
			tab,
			requested,
			browserOption(input.Format, browserop.ContentFormat("text")),
			deadline,
		)
		if itemErr != nil {
			if tab.Context().Err() != nil && environment.Context().Err() == nil && ctx.Err() == nil {
				itemErr = &cdp.BrowserError{Kind: cdp.KindTabNotFound}
			}
			switch cdp.ErrorCode(itemErr) {
			case "cancelled", "timeout", "browser_lost":
				return result, itemErr
			}
			details := cdp.ErrorDetails(itemErr)
			page = browserop.FetchedPage{
				RequestedURL: requested,
				URL:          requested,
				Error: &browserop.BrowserFailure{
					Code:    cdp.ErrorCode(itemErr),
					Message: itemErr.Error(),
					Details: &details,
				},
			}
		} else {
			end := textBoundary(page.Content, browserop.InlineBytes/max(len(input.URL), 1)/2)
			result.Truncated = result.Truncated || end < len(page.Content)
			page.Content = page.Content[:end]
		}
		result.Pages = append(result.Pages, page)
	}
	return result, nil
}

// fetchPage extracts one temporary tab while holding its command admission.
func fetchPage(
	ctx context.Context,
	tab *tabs.Tab,
	requested string,
	format browserop.ContentFormat,
	deadline time.Time,
) (browserop.FetchedPage, error) {
	result := browserop.FetchedPage{RequestedURL: requested}
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return result, &cdp.BrowserError{Kind: cdp.KindBusy}
	}
	defer checkout.Release()
	err := operation.Run(ctx, func(work context.Context) error {
		var err error
		result.URL, err = tab.Navigate(
			work,
			&tabs.Visit{URL: requested},
			"domcontentloaded",
			operation,
			&checkout.Session().References,
		)
		if err != nil {
			return err
		}
		result.Content, err = readContent(work, tab, format, &checkout.Session().References)
		if err != nil {
			return err
		}
		_, result.Title, err = targetInfo(work, tab)
		return err
	})
	return result, err
}
