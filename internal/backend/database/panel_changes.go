package database

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"slices"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapiproto"
)

// ErrPanelFull means a new tab exceeds the panel's count or size bound.
var (
	//nolint:staticcheck // This message is part of the public panel refusal.
	ErrPanelFull = errors.New("The work panel holds as many tabs, or as much data, as it can")
	// ErrPanelTooLarge means an update exceeds the panel's size bound.
	//nolint:staticcheck // This message is part of the public panel refusal.
	ErrPanelTooLarge = errors.New("The work panel's tabs would be over their size")
)

// PanelChange is one operation on a conversation's panel.
//
//sumtype:decl
type PanelChange interface{ panelChange() }

// PanelCreate inserts a new tab, unless its ID was already used.
type PanelCreate struct {
	// Tab is the new tab and its optional insertion index.
	Tab webapiproto.CreatePanelTab
}

func (PanelCreate) panelChange() {}

// PanelUpdate sets data fields and removes fields whose value is null.
type PanelUpdate struct {
	// ID identifies the tab to update.
	ID string
	// Data holds fields to set, with null values removing fields.
	Data json.RawMessage
}

func (PanelUpdate) panelChange() {}

// PanelRemove retires a tab's ID and removes the tab.
type PanelRemove struct {
	// ID identifies the tab to remove.
	ID string
}

func (PanelRemove) panelChange() {}

// PanelMove moves a tab to an index among the other tabs.
type PanelMove struct {
	// ID identifies the tab to move.
	ID string
	// Index is its new position among the other tabs.
	Index uint64
}

func (PanelMove) panelChange() {}

// PanelEffectKind identifies an applied panel operation.
type PanelEffectKind uint8

const (
	// PanelCreated means a new tab was inserted.
	PanelCreated PanelEffectKind = iota
	// PanelUpdated means data fields changed.
	PanelUpdated
	// PanelRemoved means a tab was retired.
	PanelRemoved
	// PanelMoved means a tab changed position.
	PanelMoved
)

// PanelEffect identifies the operation that changed a tab.
type PanelEffect struct {
	// Kind identifies the applied operation.
	Kind PanelEffectKind
	// Tab is the tab that was created or removed.
	Tab webapiproto.PanelTab
}

// What the backend keeps of a panel: its tabs in order, and every id it
// had, which is never used again.
// +demi:root
type panelDocument struct {
	Tabs    []webapiproto.PanelTab `json:"tabs"`
	Retired []string               `json:"retired"`
}

func (d *panelDocument) apply(change PanelChange) (PanelEffect, bool, error) {
	next := panelDocument{Tabs: slices.Clone(d.Tabs), Retired: slices.Clone(d.Retired)}
	effect, changed, err := next.change(change)
	if err != nil || !changed {
		return effect, changed, err
	}
	data, err := contract.EncodeJSON(next.Tabs)
	if err != nil {
		return PanelEffect{}, false, err
	}
	if len(data) > webapiproto.PanelBytesMax {
		if effect.Kind == PanelCreated {
			return PanelEffect{}, false, ErrPanelFull
		}
		return PanelEffect{}, false, ErrPanelTooLarge
	}
	*d = next
	return effect, true, nil
}

func (d *panelDocument) position(id string) int {
	return slices.IndexFunc(d.Tabs, func(tab webapiproto.PanelTab) bool { return tab.ID == id })
}

func (d *panelDocument) change(change PanelChange) (PanelEffect, bool, error) {
	switch c := change.(type) {
	case PanelCreate:
		return d.create(c.Tab)
	case PanelUpdate:
		return d.update(c)
	case PanelRemove:
		index := d.position(c.ID)
		if index < 0 {
			return PanelEffect{}, false, nil
		}
		tab := d.Tabs[index]
		d.Tabs = slices.Delete(d.Tabs, index, index+1)
		d.Retired = append(d.Retired, tab.ID)
		return PanelEffect{Kind: PanelRemoved, Tab: tab}, true, nil
	case PanelMove:
		from := d.position(c.ID)
		if from < 0 {
			return PanelEffect{}, false, nil
		}
		to := int(min(c.Index, uint64(len(d.Tabs)-1)))
		if from == to {
			return PanelEffect{}, false, nil
		}
		tab := d.Tabs[from]
		d.Tabs = slices.Delete(d.Tabs, from, from+1)
		d.Tabs = slices.Insert(d.Tabs, to, tab)
		return PanelEffect{Kind: PanelMoved}, true, nil
	}
	return PanelEffect{}, false, nil
}

func (d *panelDocument) create(c webapiproto.CreatePanelTab) (PanelEffect, bool, error) {
	if d.position(c.ID) >= 0 || slices.Contains(d.Retired, c.ID) {
		return PanelEffect{}, false, nil
	}
	if len(d.Tabs) >= webapiproto.PanelTabsMax {
		return PanelEffect{}, false, ErrPanelFull
	}
	tab := webapiproto.PanelTab{ID: c.ID, Kind: c.Kind, Data: bytes.Clone(c.Data)}
	index := len(d.Tabs)
	if c.Index != nil {
		index = int(min(*c.Index, uint64(index)))
	}
	d.Tabs = slices.Insert(d.Tabs, index, tab)
	return PanelEffect{Kind: PanelCreated, Tab: tab}, true, nil
}

func (d *panelDocument) update(c PanelUpdate) (PanelEffect, bool, error) {
	index := d.position(c.ID)
	if index < 0 {
		return PanelEffect{}, false, nil
	}
	fields, err := contract.ObjectFields(d.Tabs[index].Data)
	if err != nil {
		return PanelEffect{}, false, err
	}
	patch, err := contract.ObjectFields(c.Data)
	if err != nil {
		return PanelEffect{}, false, err
	}
	for _, field := range patch {
		at := slices.IndexFunc(fields, func(f contract.Field) bool { return f.Name == field.Name })
		raw, err := contract.EncodeJSON(field.Value)
		if err != nil {
			return PanelEffect{}, false, err
		}
		if bytes.Equal(raw, []byte("null")) {
			if at >= 0 {
				// Removing a field fills its position with the last field.
				fields[at] = fields[len(fields)-1]
				fields = fields[:len(fields)-1]
			}
			continue
		}
		if at >= 0 {
			fields[at] = field
		} else {
			fields = append(fields, field)
		}
	}
	data, err := contract.EncodeObject(fields)
	if err != nil {
		return PanelEffect{}, false, err
	}
	before, err := comparableJSONValue(d.Tabs[index].Data)
	if err != nil {
		return PanelEffect{}, false, err
	}
	after, err := comparableJSONValue(data)
	if err != nil {
		return PanelEffect{}, false, err
	}
	if reflect.DeepEqual(before, after) {
		return PanelEffect{}, false, nil
	}
	d.Tabs[index].Data = data
	return PanelEffect{Kind: PanelUpdated}, true, nil
}

// PanelKindError identifies a kind the caller does not own.
type PanelKindError struct {
	// Kind is the kind of the requested or existing tab.
	Kind string
}

// Error returns the panel refusal shown to the caller.
func (e *PanelKindError) Error() string {
	return fmt.Sprintf(`No plugin the user has on declares the panel kind "%s"`, e.Kind)
}

type panelScope struct {
	restricted bool
	kinds      []string
}

func (d panelDocument) checkKinds(change PanelChange, scope panelScope) error {
	if !scope.restricted {
		return nil
	}
	var id string
	switch c := change.(type) {
	case PanelCreate:
		if !slices.Contains(scope.kinds, c.Tab.Kind) {
			return &PanelKindError{Kind: c.Tab.Kind}
		}
		return nil
	case PanelUpdate:
		id = c.ID
	case PanelRemove:
		id = c.ID
	case PanelMove:
		id = c.ID
	}
	index := d.position(id)
	if index >= 0 && !slices.Contains(scope.kinds, d.Tabs[index].Kind) {
		return &PanelKindError{Kind: d.Tabs[index].Kind}
	}
	return nil
}
