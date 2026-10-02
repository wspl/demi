package main

import (
	"fmt"
	"go/types"
	"sort"
	"strings"
)

// unionSeal identifies the sole unexported contract-union method and checks its signature.
func unionSeal(t types.Type) (*types.Func, error) {
	iface, ok := t.Underlying().(*types.Interface)
	if !ok {
		return nil, fmt.Errorf("union must be an interface with one unexported sealing method")
	}
	var seal *types.Func
	for i := 0; i < iface.NumMethods(); i++ {
		method := iface.Method(i)
		if method.Exported() {
			continue
		}
		if seal != nil {
			return nil, fmt.Errorf("union must have exactly one unexported sealing method")
		}
		seal = method
	}
	if seal == nil {
		return nil, fmt.Errorf("union must have exactly one unexported sealing method")
	}
	signature := seal.Type().(*types.Signature)
	if signature.Params().Len() != 0 || signature.Results().Len() != 0 {
		return nil, fmt.Errorf("union sealing method must have no parameters or results")
	}
	return seal, nil
}

// normalizeVariants resolves contract variants to every sealing interface they implement.
func (g *generator) normalizeVariants() {
	for _, key := range g.order {
		d := g.defs[key]
		if !has(d.marks, "variant") {
			continue
		}
		parts := strings.Fields(d.marks["variant"])
		if len(parts) > 2 {
			continue
		}
		value := ""
		if len(parts) > 0 {
			value = parts[len(parts)-1]
		}
		for _, other := range g.order {
			u := g.defs[other]
			if !has(u.marks, "union") || u.typ.Obj().Pkg() != d.typ.Obj().Pkg() {
				continue
			}
			iface, ok := u.typ.Underlying().(*types.Interface)
			if !ok {
				continue
			}
			explicit := len(parts) == 2 && parts[0] == u.name
			if explicit || len(parts) < 2 && types.Implements(types.NewPointer(d.typ), iface) {
				d.unions = append(d.unions, other)
			}
		}
		if len(d.unions) == 0 && len(parts) == 1 {
			for _, other := range g.order {
				u := g.defs[other]
				if has(u.marks, "union") && u.typ.Obj().Pkg() == d.typ.Obj().Pkg() {
					d.unions = append(d.unions, other)
				}
			}
			if len(d.unions) != 1 {
				d.unions = nil
			}
		}
		if len(d.unions) > 0 {
			d.marks["variant"] = d.unions[0] + " " + value
		}
	}
}

// variants preserves declaration order for serde's first-matching untagged rule.
func (g *generator) variants(name string) []*definition {
	var out []*definition
	for _, key := range g.order {
		d := g.defs[key]
		for _, union := range d.unions {
			if union == name {
				out = append(out, d)
				break
			}
		}
	}
	if g.defs[name].marks["union"] == "untagged" {
		sort.SliceStable(out, func(i, j int) bool { return out[i].typ.Obj().Pos() < out[j].typ.Obj().Pos() })
	}
	return out
}

// variantWire defines the one standalone representation shared by a variant's unions.
func (g *generator) variantWire(d *definition) (tag, value, kind string) {
	kind = "string"
	if len(d.unions) == 0 {
		return
	}
	u := g.defs[d.unions[0]]
	tag = bounds(u.marks["union"])["tag"]
	_, value, _ = strings.Cut(d.marks["variant"], " ")
	if value == "true" || value == "false" {
		kind = "bool"
	}
	return
}

func tagLiteral(value string) string {
	if value == "true" || value == "false" {
		return value
	}
	return q(value)
}

func (g *generator) unionKind(d *definition) string {
	for _, v := range g.variants(d.key) {
		_, _, kind := g.variantWire(v)
		return kind
	}
	return "string"
}

// retainContracts limits generated methods and diagnostics to root-reachable types.
func (g *generator) retainContracts() error {
	g.received = map[string]bool{}
	for _, key := range g.order {
		if has(g.defs[key].marks, "root") {
			g.markReceived(key)
		}
	}
	if g.err != nil {
		return g.err
	}
	order := []string{}
	for _, key := range g.order {
		if g.received[key] {
			order = append(order, key)
		}
	}
	g.order = order
	for _, key := range g.order {
		d := g.defs[key]
		if has(d.marks, "variant") && len(d.unions) == 0 {
			return fmt.Errorf("%s: %s: variant requires a sealing union", d.position, d.name)
		}
		for _, union := range d.unions {
			u := g.defs[union]
			if bounds(u.marks["union"])["content"] != bounds(g.defs[d.unions[0]].marks["union"])["content"] || bounds(u.marks["union"])["tag"] != bounds(g.defs[d.unions[0]].marks["union"])["tag"] || u.marks["msgpack"] != g.defs[d.unions[0]].marks["msgpack"] {
				return fmt.Errorf("%s: %s: shared variant requires the same wire representation", d.position, d.name)
			}
		}
	}
	return nil
}

func emptyCollection(t types.Type) bool {
	switch t.Underlying().(type) {
	case *types.Map, *types.Slice:
		return true
	}
	return false
}
