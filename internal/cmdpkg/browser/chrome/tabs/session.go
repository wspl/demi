package tabs

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

// Session is mutable command data, accessed only through an exclusive Checkout.
// Zero values of its collections are ready for use through their methods.
type Session struct {
	References References
	Assets     Assets
	WebMCP     cdp.WebMCPState
}

// Gate admits one command without queuing. Its zero value is ready for use.
type Gate struct{}

// TryCheckout returns nil while another command holds the session.
func (g *Gate) TryCheckout() *Checkout { panic("not written: k-chrome-tabs") }

// Checkout leases the tab session. Its owner defers Release at acquisition.
type Checkout struct{}

// Session returns the exclusively held data, valid only until Release.
func (c *Checkout) Session() *Session { panic("not written: k-chrome-tabs") }

// Release returns the session to its gate; repeated release is harmless.
func (c *Checkout) Release() { panic("not written: k-chrome-tabs") }

// Reference identifies a node within the document in which it was observed.
type Reference struct {
	Backend protocol.BackendNodeID
	Frame   protocol.FrameID
	Loader  protocol.LoaderID
}

// References numbers nodes monotonically within a tab. Its zero value is ready.
type References struct{}

// Invalidate forgets old document references without reusing their numbers.
func (r *References) Invalidate() { panic("not written: k-chrome-tabs") }

// Retain keeps only references whose node the predicate accepts.
func (r *References) Retain(keep func(Reference) bool) { panic("not written: k-chrome-tabs") }

// Lookup returns a retained node and whether the reference is still known.
func (r *References) Lookup(id browserop.NodeRef) (Reference, bool) {
	panic("not written: k-chrome-tabs")
}

// Issue returns an existing node reference or the next number, subject to Rust's
// 10,000-reference document limit.
func (r *References) Issue(reference Reference) (browserop.NodeRef, error) {
	panic("not written: k-chrome-tabs")
}

// Assets holds document-scoped inventories under the tab's command gate.
// Callers initialize nil maps when first storing entries.
type Assets struct {
	Documents   map[protocol.FrameID]protocol.LoaderID
	Inventories map[string]Inventory
}

// Inventory holds the assets returned by one listing.
type Inventory struct{ Assets []Asset }

// Asset holds an inventoried resource and where its bytes can be obtained.
// This is internal tab state, not a duplicate browser wire contract.
type Asset struct {
	ID     string
	Kind   browserop.AssetKind
	MIME   string
	Source AssetSource
}

// AssetSource identifies the source of an inventory item's bytes.
//
//sumtype:decl
type AssetSource interface{ assetSource() }

// ResourceAsset retrieves a URL through the renderer owning its frame.
type ResourceAsset struct {
	Page  cdp.FrameTarget
	Frame protocol.FrameID
	URL   string
}

func (*ResourceAsset) assetSource() {}

// SVGAsset holds an inline SVG document.
type SVGAsset struct{ SVG string }

func (*SVGAsset) assetSource() {}
