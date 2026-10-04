package plugintest

import (
	"bytes"
	"maps"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/plugin"
)

// hostFile reads a test Host file or lists its implicit parent directories.
func hostFile(files map[string][]byte, read plugin.HostRead) plugin.HostFile {
	if data, ok := files[read.Path]; ok {
		limit := min(uint64(len(data)), read.Limit)
		return &plugin.HostFileFile{Bytes: bytes.Clone(data[:limit]), Size: uint64(len(data))}
	}
	prefix := strings.TrimRight(read.Path, "/") + "/"
	entries := []plugin.HostEntry{}
	for _, path := range slices.Sorted(maps.Keys(files)) {
		rest, ok := strings.CutPrefix(path, prefix)
		if !ok {
			continue
		}
		name, _, directory := strings.Cut(rest, "/")
		kind := plugin.EntryKindFile
		if directory {
			kind = plugin.EntryKindDirectory
		}
		if !slices.ContainsFunc(entries, func(e plugin.HostEntry) bool {
			return e.Name == name
		}) {
			entries = append(entries, plugin.HostEntry{Name: name, Kind: kind})
		}
	}
	if len(entries) == 0 {
		return &plugin.HostFileMissing{}
	}
	return &plugin.HostFileDirectory{Entries: entries}
}
