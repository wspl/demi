package page

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/network"
	chrome "github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
)

type inventoryFrame struct {
	page cdp.FrameTarget
	tree *chrome.FrameResourceTree
}

// inventoryFrames locates each frame's resource tree on its owning renderer.
func inventoryFrames(ctx context.Context, page cdp.FrameTarget) ([]inventoryFrame, error) {
	snapshot, err := cdp.CaptureFrames(ctx, page)
	if err != nil {
		return nil, err
	}
	frames := make([]inventoryFrame, 0, len(snapshot.Frames))
	for _, document := range snapshot.Frames {
		tree, err := chrome.GetResourceTree().Do(protocol.WithExecutor(ctx, document.Target))
		if err != nil {
			return nil, err
		}
		pending := []*chrome.FrameResourceTree{tree}
		var found *chrome.FrameResourceTree
		for len(pending) > 0 {
			frame := pending[len(pending)-1]
			pending = pending[:len(pending)-1]
			pending = append(pending, frame.ChildFrames...)
			if frame.Frame.ID == document.Frame.ID {
				found = frame
				break
			}
		}
		if found == nil {
			return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "asset frame has no resource tree"}
		}
		frames = append(frames, inventoryFrame{document.Target, found})
	}
	return frames, nil
}

// inventoryDocuments invalidates inventories after any captured frame changes documents.
func inventoryDocuments(state *tabs.Assets, frames []inventoryFrame) {
	documents := map[protocol.FrameID]protocol.LoaderID{}
	for _, frame := range frames {
		documents[frame.tree.Frame.ID] = frame.tree.Frame.LoaderID
	}
	if !maps.Equal(documents, state.Documents) {
		state.Inventories = nil
		state.Documents = documents
	}
	if state.Inventories == nil {
		state.Inventories = map[string]tabs.Inventory{}
	}
}

// AssetsList inventories resources already observed by the page.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func AssetsList(
	ctx context.Context,
	_ *cmdsdk.InvocationContext[cmdproto.Invocation],
	tab *tabs.Tab,
	_ browserproto.AssetsListInput,
	deadline time.Time,
) (result browserproto.AssetsListResult, err error) {
	if tab == nil {
		return result, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return result, &cdp.BrowserError{Kind: cdp.KindBusy}
	}
	defer checkout.Release()
	err = operation.Run(ctx, func(work context.Context) error {
		frames, err := inventoryFrames(work, tab.Page())
		if err != nil {
			return err
		}
		state := &checkout.Session().Assets
		inventoryDocuments(state, frames)
		handle, err := cdp.Fresh("assets")
		if err != nil {
			return err
		}
		result = browserproto.AssetsListResult{
			Inventory:  handle,
			Assets:     []browserproto.Asset{},
			InlineSVGs: []browserproto.InlineSVG{},
		}
		inventory := tabs.Inventory{Assets: []tabs.Asset{}}
		size := 0
		observation, err := captureObservation(work, tab.Page(), &checkout.Session().References)
		if err != nil {
			return err
		}
		for _, frame := range frames {
			if err := inventoryResources(frame, &inventory, &result, &size); err != nil {
				return err
			}
			if err := inventorySVGs(work, frame, observation, &inventory, &result, &size); err != nil {
				return err
			}
		}
		state.Inventories[handle] = inventory
		return nil
	})
	// The Go implementation resolves document objects for frame-local extraction.
	err = cdp.AfterCleanup(err, releaseObjects(ctx, tab))
	return result, err
}

type exportFailure struct {
	// ID identifies the asset that could not be exported.
	ID string `json:"id"`
	// Error describes the export failure.
	Error browserproto.BrowserFailure `json:"error"`
}
type exportManifest struct {
	// Inventory identifies the inventory used for this export.
	Inventory string `json:"inventory"`
	// Files lists the successfully exported assets.
	Files []browserproto.ExportedAsset `json:"files"`
	// Failures lists the assets that could not be exported.
	Failures []exportFailure `json:"failures"`
}

// AssetsExport exports resources from the selected page inventory.
// The operation owns its admission and cleanup; paths resolve against invocation metadata.
func AssetsExport(
	ctx context.Context,
	invocation *cmdsdk.InvocationContext[cmdproto.Invocation],
	tab *tabs.Tab,
	input browserproto.AssetsExportInput,
	deadline time.Time,
) (result browserproto.AssetsExportResult, err error) {
	if tab == nil {
		return result, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return result, &cdp.BrowserError{Kind: cdp.KindBusy}
	}
	defer checkout.Release()
	var frames []inventoryFrame
	err = operation.Run(ctx, func(work context.Context) error {
		var err error
		frames, err = inventoryFrames(work, tab.Page())
		return err
	})
	if err != nil {
		return result, err
	}
	state := &checkout.Session().Assets
	inventoryDocuments(state, frames)
	inventory, err := exportInventory(state, input)
	if err != nil {
		return result, err
	}
	directory, err := cdp.Resolve(invocation.Request.Cwd, input.OutputDir)
	if err != nil {
		return result, err
	}
	if err = os.MkdirAll(directory, 0o755); err != nil {
		return result, &cdp.BrowserError{Kind: cdp.KindIO, Cause: err}
	}
	manifestPath := filepath.Join(directory, "manifest.json")
	overwrite := browserOption(input.Overwrite, false)
	if _, err = cdp.Preflight(operation.Context(), invocation.Request.Cwd, manifestPath, overwrite); err != nil {
		return result, err
	}
	manifest := exportAssets(ctx, operation, invocation.Request.Cwd, directory, input, inventory, overwrite)
	manifestBytes, err := encodeAssetManifest(manifest)
	if err != nil {
		return result, err
	}
	cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
	defer cancel()
	path, err := cdp.SaveWithOverwrite(cleanup, invocation.Request.Cwd, manifestPath, manifestBytes, overwrite)
	if err != nil {
		return result, err
	}
	result = browserproto.AssetsExportResult{Directory: directory, Manifest: path, Files: manifest.Files}
	return assetExportResult(ctx, deadline, result, manifest)
}

// assetExtension names exported resources from their declared media type.
func assetExtension(mime string) string {
	switch mime {
	case "image/png":
		return "png"
	case "image/jpeg":
		return "jpg"
	case "image/gif":
		return "gif"
	case "image/webp":
		return "webp"
	case "image/svg+xml":
		return "svg"
	case "text/css":
		return "css"
	case "font/woff":
		return "woff"
	case "font/woff2":
		return "woff2"
	case "font/ttf":
		return "ttf"
	case "video/mp4":
		return "mp4"
	case "video/webm":
		return "webm"
	}
	return "bin"
}

// resourceBytes reads an already-observed resource without navigating its frame.
func resourceBytes(ctx context.Context, page cdp.FrameTarget, frame protocol.FrameID, url string) ([]byte, error) {
	data, err := chrome.GetResourceContent(frame, url).Do(protocol.WithExecutor(ctx, page))
	var corrupt base64.CorruptInputError
	if errors.As(err, &corrupt) {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	return data, err
}

func inventoryResources(
	frame inventoryFrame,
	inventory *tabs.Inventory,
	result *browserproto.AssetsListResult,
	size *int,
) error {
	for _, resource := range frame.tree.Resources {
		var kind browserproto.AssetKind
		switch resource.Type {
		case network.ResourceTypeFont:
			kind = "font"
		case network.ResourceTypeImage:
			kind = "image"
		case network.ResourceTypeStylesheet:
			kind = "stylesheet"
		case network.ResourceTypeMedia:
			if strings.HasPrefix(resource.MimeType, "video/") {
				kind = "video"
			}
		}
		if kind == "" {
			continue
		}
		id, err := cdp.Fresh("asset")
		if err != nil {
			return err
		}
		entry := browserproto.Asset{ID: id, Kind: kind, URL: resource.URL, MIMEType: &resource.MimeType}
		encoded, err := cdp.Value(entry)
		if err != nil {
			return err
		}
		if len(inventory.Assets) >= browserproto.MaxNodes || *size+len(encoded) > browserproto.InlineBytes/2 {
			result.Truncated = true
			continue
		}
		*size += len(encoded)
		result.Assets = append(result.Assets, entry)
		inventory.Assets = append(
			inventory.Assets,
			tabs.Asset{
				ID:     id,
				Kind:   kind,
				MIME:   resource.MimeType,
				Source: &tabs.ResourceAsset{Page: frame.page, Frame: frame.tree.Frame.ID, URL: resource.URL},
			},
		)
	}

	return nil
}

func inventorySVGs(
	work context.Context,
	frame inventoryFrame,
	observation *observation,
	inventory *tabs.Inventory,
	result *browserproto.AssetsListResult,
	size *int,
) error {
	var document *targetElement
	for _, item := range observation.dom {
		if item.frame == frame.tree.Frame.ID && item.node.NodeType == 9 {
			resolved, err := resolveElement(work, frame.page, item.node.BackendNodeID)
			if err != nil {
				return err
			}
			document = &resolved
			break
		}
	}
	if document == nil {
		return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "asset frame has no execution context"}
	}
	svgs, err := decodeElement[[]string](
		work,
		*document,
		"function() { return Array.from(document.querySelectorAll('svg'), svg => svg.outerHTML); }",
		false,
	)
	if err != nil {
		return err
	}
	for _, html := range svgs {
		if len(inventory.Assets) >= browserproto.MaxNodes || *size+len(html) > browserproto.InlineBytes/2 {
			result.Truncated = true
			continue
		}
		*size += len(html)
		id, err := cdp.Fresh("asset")
		if err != nil {
			return err
		}
		result.InlineSVGs = append(result.InlineSVGs, browserproto.InlineSVG{ID: id, HTML: html})
		inventory.Assets = append(
			inventory.Assets,
			tabs.Asset{ID: id, Kind: "image", MIME: "image/svg+xml", Source: &tabs.SVGAsset{SVG: html}},
		)
	}
	return nil
}

func validateAssetIDs(input browserproto.AssetsExportInput, inventory tabs.Inventory) error {
	if input.ID != nil {
		for _, id := range *input.ID {
			found := false
			for _, asset := range inventory.Assets {
				found = found || asset.ID == id
			}
			if !found {
				return &cdp.BrowserError{
					Kind:    cdp.KindConfiguration,
					Message: "asset " + id + " is not in this inventory",
				}
			}
		}
	}

	return nil
}

func exportInventory(state *tabs.Assets, input browserproto.AssetsExportInput) (tabs.Inventory, error) {
	if (input.ID != nil) == (input.Kind != nil) {
		return tabs.Inventory{}, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "asset export requires either --id or --kind",
		}
	}
	inventory, ok := state.Inventories[input.Inventory]
	if !ok {
		return tabs.Inventory{}, &cdp.BrowserError{Kind: cdp.KindStaleInventory}
	}
	if err := validateAssetIDs(input, inventory); err != nil {
		return tabs.Inventory{}, err
	}

	return inventory, nil
}

func encodeAssetManifest(manifest exportManifest) ([]byte, error) {
	encoded, err := cdp.Value(manifest)
	if err != nil {
		return nil, err
	}
	var pretty bytes.Buffer
	if err = json.Indent(&pretty, encoded, "", "  "); err != nil {
		return nil, err
	}
	return pretty.Bytes(), nil
}

func exportAssets(
	ctx context.Context,
	operation *cdp.Operation,
	cwd, directory string,
	input browserproto.AssetsExportInput,
	inventory tabs.Inventory,
	overwrite bool,
) exportManifest {
	manifest := exportManifest{
		Inventory: input.Inventory,
		Files:     []browserproto.ExportedAsset{},
		Failures:  []exportFailure{},
	}
	for _, asset := range inventory.Assets {
		selected := input.ID != nil && slices.Contains(*input.ID, asset.ID) ||
			input.Kind != nil && slices.Contains(*input.Kind, asset.Kind)
		if !selected {
			continue
		}
		var data []byte
		var saved string
		itemErr := operation.Run(ctx, func(work context.Context) error {
			var err error
			switch source := asset.Source.(type) {
			case *tabs.SVGAsset:
				data = []byte(source.SVG)
			case *tabs.ResourceAsset:
				data, err = resourceBytes(work, source.Page, source.Frame, source.URL)
			}
			if err != nil {
				return err
			}
			saved, err = cdp.SaveWithOverwrite(
				work,
				cwd,
				filepath.Join(directory, asset.ID+"."+assetExtension(asset.MIME)),
				data,
				overwrite,
			)
			return err
		})
		if itemErr != nil {
			manifest.Failures = append(
				manifest.Failures,
				exportFailure{
					asset.ID,
					browserproto.BrowserFailure{Code: cdp.ErrorCode(itemErr), Message: itemErr.Error()},
				},
			)
		} else {
			manifest.Files = append(
				manifest.Files,
				browserproto.ExportedAsset{ID: asset.ID, Path: saved, Bytes: uint(len(data)), MIMEType: asset.MIME},
			)
		}
	}
	return manifest
}

func assetExportResult(
	ctx context.Context,
	deadline time.Time,
	result browserproto.AssetsExportResult,
	manifest exportManifest,
) (browserproto.AssetsExportResult, error) {
	if ctx.Err() != nil || !time.Now().Before(deadline) {
		kind := cdp.KindCancelled
		if !time.Now().Before(deadline) {
			kind = cdp.KindTimeout
		}
		return result, &cdp.BrowserError{
			Kind:    cdp.KindAction,
			Cause:   &cdp.BrowserError{Kind: kind},
			Details: browserproto.ErrorDetails{AssetsExportResult: &result},
		}
	}
	if len(manifest.Failures) > 0 {
		return result, &cdp.BrowserError{
			Kind:    cdp.KindPartialFailure,
			Details: browserproto.ErrorDetails{AssetsExportResult: &result},
		}
	}
	return result, nil
}
