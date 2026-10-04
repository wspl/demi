package store

import (
	"reflect"

	"github.com/wspl/demi/internal/types"
)

// cloneModelBlocks freezes transcript records, including nested model selections
// and tool views. slices.Clone and maps.Clone are shallow; the workspace has no
// deep copier. Reflection preserves the sealed contract variant without a second
// declaration or per-variant copy implementation that can omit a new field.
// This copies typed in-process contract data, never decodes outside input.
func cloneModelBlocks(blocks []types.Block) []types.Block {
	if blocks == nil {
		return nil
	}
	cloned := make([]types.Block, len(blocks))
	for index, block := range blocks {
		if block != nil {
			reflect.ValueOf(&cloned[index]).Elem().Set(cloneTranscriptValue(reflect.ValueOf(block)))
		}
	}
	return cloned
}

// cloneTranscriptValue copies the value shapes permitted in transcript contracts.
func cloneTranscriptValue(value reflect.Value) reflect.Value {
	switch value.Kind() {
	case reflect.Pointer:
		if value.IsNil() {
			return reflect.Zero(value.Type())
		}
		cloned := reflect.New(value.Type().Elem())
		cloned.Elem().Set(cloneTranscriptValue(value.Elem()))
		return cloned
	case reflect.Interface:
		if value.IsNil() {
			return reflect.Zero(value.Type())
		}
		cloned := reflect.New(value.Type()).Elem()
		cloned.Set(cloneTranscriptValue(value.Elem()))
		return cloned
	case reflect.Slice:
		if value.IsNil() {
			return reflect.Zero(value.Type())
		}
		cloned := reflect.MakeSlice(value.Type(), value.Len(), value.Len())
		for index := 0; index < value.Len(); index++ {
			cloned.Index(index).Set(cloneTranscriptValue(value.Index(index)))
		}
		return cloned
	case reflect.Map:
		if value.IsNil() {
			return reflect.Zero(value.Type())
		}
		cloned := reflect.MakeMapWithSize(value.Type(), value.Len())
		entries := value.MapRange()
		for entries.Next() {
			cloned.SetMapIndex(entries.Key(), cloneTranscriptValue(entries.Value()))
		}
		return cloned
	case reflect.Struct:
		cloned := reflect.New(value.Type()).Elem()
		for index := 0; index < value.NumField(); index++ {
			cloned.Field(index).Set(cloneTranscriptValue(value.Field(index)))
		}
		return cloned
	default:
		return value
	}
}
