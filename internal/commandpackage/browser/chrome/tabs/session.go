package tabs

import (
	"strconv"
	"sync"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
)

// Session is mutable command data, accessed only through an exclusive Checkout.
// Zero values of its collections are ready for use through their methods.
type Session struct {
	// References owns document-bound node references.
	References References
	// Assets owns captured asset inventories.
	Assets Assets
	// WebMCP owns document-bound tool declarations.
	WebMCP cdp.WebMCPState
}

// Gate admits one command without queuing. Its zero value is ready for use.
type Gate struct {
	// mu protects admission and the retained session, never the command itself.
	mu      sync.Mutex
	held    bool
	session Session
}

// TryCheckout returns nil while another command holds the session.
func (g *Gate) TryCheckout() *Checkout {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.held {
		return nil
	}
	g.held = true
	return &Checkout{gate: g}
}

// Checkout leases the tab session. Its owner defers Release at acquisition.
type Checkout struct {
	gate *Gate
	once sync.Once
}

// Session returns the exclusively held data, valid only until Release.
func (c *Checkout) Session() *Session {
	return &c.gate.session
}

// Release returns the session to its gate; repeated release is harmless.
func (c *Checkout) Release() {
	c.once.Do(func() {
		c.gate.mu.Lock()
		c.gate.held = false
		c.gate.mu.Unlock()
	})
}

// Reference identifies a node within the document in which it was observed.
type Reference struct {
	// Backend identifies the Chrome DOM node.
	Backend protocol.BackendNodeID
	// Frame identifies the node's document frame.
	Frame protocol.FrameID
	// Loader binds the node to its document loader.
	Loader protocol.LoaderID
}

// References numbers nodes monotonically within a tab. Its zero value is ready.
type References struct {
	byID   map[browserproto.NodeRef]Reference
	byNode map[Reference]browserproto.NodeRef
	last   uint64
}

// Invalidate forgets old document references without reusing their numbers.
func (r *References) Invalidate() {
	clear(r.byID)
	clear(r.byNode)
}

// Retain keeps only references whose node the predicate accepts.
func (r *References) Retain(keep func(Reference) bool) {
	for id, reference := range r.byID {
		if !keep(reference) {
			delete(r.byID, id)
			delete(r.byNode, reference)
		}
	}
}

// Lookup returns a retained node and whether the reference is still known.
func (r *References) Lookup(id browserproto.NodeRef) (Reference, bool) {
	node, ok := r.byID[id]
	return node, ok
}

// Issue returns an existing node reference or the next number, subject to a
// 10,000-reference document limit.
func (r *References) Issue(reference Reference) (browserproto.NodeRef, error) {
	if id, ok := r.byNode[reference]; ok {
		return id, nil
	}
	if len(r.byID) >= 10000 {
		return "", &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "browser reference limit reached for this document",
		}
	}
	if r.byID == nil {
		r.byID = make(map[browserproto.NodeRef]Reference)
		r.byNode = make(map[Reference]browserproto.NodeRef)
	}
	r.last++
	id := browserproto.NodeRef("e" + strconv.FormatUint(r.last, 10))
	r.byID[id] = reference
	r.byNode[reference] = id
	return id, nil
}

// Assets holds document-scoped inventories under the tab's command gate.
// Callers initialize nil maps when first storing entries.
type Assets struct {
	// Documents tracks the loaders captured by the inventories.
	Documents map[protocol.FrameID]protocol.LoaderID
	// Inventories indexes inventories by their opaque handles.
	Inventories map[string]Inventory
}

// Inventory holds the assets returned by one listing.
type Inventory struct {
	// Assets retains the captured resource and SVG entries.
	Assets []Asset
}

// Asset holds an inventoried resource and where its bytes can be obtained.
// This is internal tab state, not a duplicate browser wire contract.
type Asset struct {
	// ID identifies the entry within its inventory.
	ID string
	// Kind classifies the captured resource.
	Kind browserproto.AssetKind
	// MIME holds the observed media type.
	MIME string
	// Source retains the renderer resource or inline SVG.
	Source AssetSource
}

// AssetSource identifies the source of an inventory item's bytes.
//
//sumtype:decl
type AssetSource interface{ assetSource() }

// ResourceAsset retrieves a URL through the renderer owning its frame.
type ResourceAsset struct {
	// Page executes commands in the resource's renderer.
	Page cdp.FrameTarget
	// Frame identifies the resource's document frame.
	Frame protocol.FrameID
	// URL names the observed resource.
	URL string
}

func (*ResourceAsset) assetSource() {}

// SVGAsset holds an inline SVG document.
type SVGAsset struct {
	// SVG retains the captured SVG markup.
	SVG string
}

func (*SVGAsset) assetSource() {}
