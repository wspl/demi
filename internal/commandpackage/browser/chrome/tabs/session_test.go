package tabs_test

import (
	"errors"
	"testing"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
)

func TestCommandGateDoesNotQueueAndReferencesNeverRecycle(t *testing.T) {
	var gate tabs.Gate
	first := gate.TryCheckout()
	if first == nil || gate.TryCheckout() != nil {
		t.Fatal("gate did not exclude competing command")
	}
	references := &first.Session().References
	node := tabs.Reference{Backend: 7, Frame: "main", Loader: "before"}
	id, err := references.Issue(node)
	if err != nil {
		t.Fatal(err)
	}
	repeated, err := references.Issue(node)
	if err != nil || id != repeated {
		t.Fatal(repeated, err)
	}
	references.Invalidate()
	if _, ok := references.Lookup(id); ok {
		t.Fatal("stale reference retained")
	}
	newer, err := references.Issue(node)
	if err != nil || newer == id {
		t.Fatal(newer, err)
	}
	first.Release()
	first.Release()
	second := gate.TryCheckout()
	if second == nil {
		t.Fatal("released gate stayed occupied")
	}
	defer second.Release()
	if _, ok := second.Session().References.Lookup(newer); !ok {
		t.Fatal("session was not returned")
	}
	second.Session().References.Retain(func(tabs.Reference) bool { return false })
	if _, ok := references.Lookup(newer); ok {
		t.Fatal("discarded node retained")
	}
}

// References are mutable only under the checkout, independently from other tabs.
func TestReferenceDocumentLimit(t *testing.T) {
	var references tabs.References
	for index := range 10000 {
		if _, err := references.Issue(tabs.Reference{Backend: protocol.BackendNodeID(index)}); err != nil {
			t.Fatal(err)
		}
	}
	_, err := references.Issue(tabs.Reference{Backend: 10001})
	var failure *cdp.BrowserError
	if !errors.As(err, &failure) || failure.Kind != cdp.KindConfiguration {
		t.Fatal(err)
	}
	references.Invalidate()
	if _, err := references.Issue(tabs.Reference{Backend: 10001}); err != nil {
		t.Fatal(err)
	}
}
