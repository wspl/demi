package main

import (
	"fmt"
	"go/token"
	"go/types"
)

// checkCodec verifies the methods invoked by contract's JSON and MessagePack
// encoders and decoders, without interpreting the codec's private representation.
func checkCodec(d *definition, msgpack bool) error {
	if _, ok := d.typ.Underlying().(*types.Interface); ok {
		return fmt.Errorf("codec requires a concrete named type")
	}
	if d.marks["codec"] == "string" && !types.Identical(d.typ.Underlying(), types.Typ[types.String]) {
		return fmt.Errorf("codec string requires a string underlying type")
	}
	for marker := range d.marks {
		switch marker {
		case "codec", "root", "msgpack", "schema":
		case "pattern", "length", "format":
			if d.marks["codec"] != "string" {
				return fmt.Errorf("codec string is required for %s", marker)
			}
		default:
			return fmt.Errorf("codec cannot be combined with %s", marker)
		}
	}
	formats := []string{"JSON"}
	if msgpack {
		formats = append(formats, "Msgpack")
	}
	bytes := types.NewVar(token.NoPos, nil, "", types.NewSlice(types.Typ[types.Byte]))
	err := types.NewVar(token.NoPos, nil, "", types.Universe.Lookup("error").Type())
	for _, format := range formats {
		for _, method := range []struct {
			name     string
			receiver types.Type
			params   *types.Tuple
			results  *types.Tuple
		}{
			{"Marshal" + format, d.typ, nil, types.NewTuple(bytes, err)},
			{"Unmarshal" + format, types.NewPointer(d.typ), types.NewTuple(bytes), types.NewTuple(err)},
		} {
			signature := types.NewSignatureType(nil, nil, nil, method.params, method.results, false)
			iface := types.NewInterfaceType([]*types.Func{types.NewFunc(token.NoPos, nil, method.name, signature)}, nil).Complete()
			if !types.Implements(method.receiver, iface) {
				return fmt.Errorf("codec requires %s to implement %s%s", method.receiver, method.name, types.TypeString(signature, nil)[4:])
			}
		}
	}
	return nil
}
