package tabs

import (
	"errors"
	"reflect"
	"testing"

	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
)

// registryFixture supplies only inert tab identity; tests observe registration,
// ordering and admission without Chrome, clocks or goroutines.
func registryFixture(t *testing.T, names ...string) *registryBook {
	t.Helper()
	b := newRegistryBook()
	for _, name := range names {
		id := target.ID(name)
		if !b.admit() {
			t.Fatal("creation refused")
		}
		public := b.name(id, uint64(len(b.publicIDs)+1))
		b.setUp(id)
		tab := &Tab{id: public}
		if err := b.ready(id, true, tab); err != nil {
			t.Fatal(err)
		}
	}
	return b
}

func registrySighting(id, opener string) *target.Info {
	return &target.Info{TargetID: target.ID(id), Type: "page", URL: "about:blank", OpenerID: target.ID(opener)}
}

func registryNames(b *registryBook) []browserproto.TabID {
	listed, _ := b.listing()
	names := make([]browserproto.TabID, 0, len(listed))
	for _, entry := range listed {
		names = append(names, entry.Tab.ID())
	}
	return names
}

func TestOpenersPopupsAreAnsweredOnceRegistered(t *testing.T) {
	b := registryFixture(t, "a")
	if ids, ready := b.popups("a"); !ready || len(ids) != 0 {
		t.Fatal(ids, ready)
	}
	if b.found(registrySighting("popup", "a")) {
		t.Fatal("new popup reported as changed")
	}
	id := b.name("popup", 2)
	if _, ready := b.popups("a"); ready {
		t.Fatal("pending popup answered")
	}
	if got := b.opened("a"); !reflect.DeepEqual(got, []browserproto.TabID{id}) {
		t.Fatal(got)
	}
	b.setUp("popup")
	if _, ready := b.popups("a"); ready {
		t.Fatal("setup popup answered")
	}
	if err := b.ready("popup", false, &Tab{id: id}); err != nil {
		t.Fatal(err)
	}
	if got, ready := b.popups("a"); !ready || !reflect.DeepEqual(got, []browserproto.TabID{id}) {
		t.Fatal(got, ready)
	}
	b.found(registrySighting("broken", "a"))
	b.name("broken", 3)
	b.setUp("broken")
	if err := b.ready("broken", false, nil); err != nil {
		t.Fatal(err)
	}
	b.found(registrySighting("unnamed", "a"))
	b.unusable("unnamed")
	if got, ready := b.popups("a"); !ready || !reflect.DeepEqual(got, []browserproto.TabID{id}) {
		t.Fatal(got, ready)
	}
}

func TestDestroyedBeforeCreationStaysOut(t *testing.T) {
	b := registryFixture(t, "a")
	if tab, changed := b.gone("popup"); tab != nil || changed {
		t.Fatal(tab, changed)
	}
	if b.found(registrySighting("popup", "a")) || b.pending("popup") {
		t.Fatal("destroyed target resurrected")
	}
	err := b.ready("popup", false, &Tab{id: "t2"})
	var failure *cdp.BrowserError
	if !errors.As(err, &failure) || failure.Kind != cdp.KindTabNotFound {
		t.Fatal(err)
	}
	if got := registryNames(b); !reflect.DeepEqual(got, []browserproto.TabID{"t1"}) {
		t.Fatal(got)
	}
}

func TestLastTabWaitsForCreationsBatchesAndPopups(t *testing.T) {
	b := registryFixture(t, "a")
	if !b.only("a", 0) || b.only("a", 1) {
		t.Fatal("batch hold ignored")
	}
	if !b.admit() || b.only("a", 0) {
		t.Fatal("creation ignored")
	}
	b.failed()
	if !b.only("a", 0) {
		t.Fatal("failed creation retained")
	}
	b.found(registrySighting("stray", ""))
	if !b.only("a", 0) {
		t.Fatal("unowned pending page held environment")
	}
	b.found(registrySighting("popup", "a"))
	if b.only("a", 0) {
		t.Fatal("pending popup ignored")
	}
}

func TestRegistrySealsOnceWhenLastTabGoes(t *testing.T) {
	b := registryFixture(t, "a")
	if b.settle(0) {
		t.Fatal("live tab lost")
	}
	tab, _ := b.gone("a")
	if tab == nil || tab.ID() != "t1" {
		t.Fatal("missing removed tab")
	}
	if b.settle(1) || !b.settle(0) || b.settle(0) || b.admit() {
		t.Fatal("incorrect final-tab admission")
	}
}

func TestTabReadiedAfterSealingIsRefused(t *testing.T) {
	b := registryFixture(t, "a")
	if !b.admit() {
		t.Fatal("creation refused")
	}
	b.setUp("late")
	b.gone("a")
	b.sealed = true
	err := b.ready("late", true, &Tab{id: "t2"})
	var failure *cdp.BrowserError
	if !errors.As(err, &failure) || failure.Kind != cdp.KindClosed {
		t.Fatal(err)
	}
	if len(registryNames(b)) != 0 || b.creating != 0 {
		t.Fatal("late tab retained")
	}
}

func TestListingWaitsForSetupAndKeepsCreationOrder(t *testing.T) {
	b := registryFixture(t, "first")
	b.found(registrySighting("popup", "first"))
	id := b.name("popup", 2)
	b.setUp("popup")
	if tabs, registering := b.listing(); !registering || len(tabs) != 1 {
		t.Fatal(tabs, registering)
	}
	if err := b.ready("popup", false, &Tab{id: id}); err != nil {
		t.Fatal(err)
	}
	if _, registering := b.listing(); registering {
		t.Fatal("setup retained")
	}
	if got := registryNames(b); !reflect.DeepEqual(got, []browserproto.TabID{"t1", "t2"}) {
		t.Fatal(got)
	}
}

func TestOpenerKeepsIDAfterClosing(t *testing.T) {
	b := registryFixture(t, "opener")
	public := b.publicIDs["opener"]
	b.found(registrySighting("popup", "opener"))
	b.gone("opener")
	b.found(registrySighting("popup", ""))
	if b.openers["popup"] != "opener" || b.publicIDs["opener"] != public {
		t.Fatal("lost opener identity")
	}
}

func TestReconcileFindsMissedPagesAndVanishedTargets(t *testing.T) {
	b := registryFixture(t, "a", "b")
	b.found(registrySighting("popup", "a"))
	absent := b.vanished(map[target.ID]bool{"a": true, "popup": true})
	var pending []target.ID
	for id := range b.entries {
		if b.pending(id) {
			pending = append(pending, id)
		}
	}
	if !reflect.DeepEqual(absent, []target.ID{"b"}) || !reflect.DeepEqual(pending, []target.ID{"popup"}) {
		t.Fatal(absent)
	}
}

func TestClosingTabCanBeRetriedOnlyAfterRefusal(t *testing.T) {
	b := registryFixture(t, "a", "b")
	closed := b.startClosing("a")
	if closed == nil || closed.ID() != "t1" || b.closable("a") || b.startClosing("a") != nil {
		t.Fatal("duplicate closure admitted")
	}
	b.notClosed("a")
	if !b.closable("a") {
		t.Fatal("refused close cannot retry")
	}
}
