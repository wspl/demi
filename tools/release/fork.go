package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"

	"github.com/sergi/go-diff/diffmatchpatch"
	"github.com/wspl/demi/internal/artifacts"
	"golang.org/x/mod/modfile"
	"golang.org/x/mod/module"
	"golang.org/x/mod/sumdb"
	"golang.org/x/mod/sumdb/dirhash"
	modzip "golang.org/x/mod/zip"
)

const checksumKey = "sum.golang.org+033de0ae+Ac4zctda0e5eza+HJyk9SxEdh+s3Ux18htTTAD8OuAn8"

// checksumOps keeps the authenticated log state for this comparison. The sumdb
// client verifies signatures and inclusion/consistency proofs; cache misses fetch
// fresh data. Its concurrent calls share a context owned by the command.
type checksumOps struct {
	ctx      context.Context
	client   *artifacts.Client
	url, key string
	mu       sync.Mutex
	latest   []byte
}

func (o *checksumOps) ReadRemote(path string) ([]byte, error) {
	var b bytes.Buffer
	_, err := artifacts.DownloadMeasured(o.ctx, o.client, o.url+path, 64*1024*1024, &b)
	return b.Bytes(), err
}

func (o *checksumOps) ReadConfig(name string) ([]byte, error) {
	if name == "key" {
		return []byte(o.key), nil
	}
	o.mu.Lock()
	defer o.mu.Unlock()
	return bytes.Clone(o.latest), nil
}

func (o *checksumOps) WriteConfig(_ string, old, next []byte) error {
	o.mu.Lock()
	defer o.mu.Unlock()
	if !bytes.Equal(old, o.latest) {
		return sumdb.ErrWriteConflict
	}
	o.latest = bytes.Clone(next)
	return nil
}
func (*checksumOps) ReadCache(string) ([]byte, error) { return nil, os.ErrNotExist }
func (*checksumOps) WriteCache(string, []byte)        {}
func (*checksumOps) Log(string)                       {}

// SecurityError is returned as sumdb.ErrSecurity by Lookup; it is never ignored.
func (*checksumOps) SecurityError(string) {}

func (a *application) fork(ctx context.Context, patch bool) (err error) {
	data, err := os.ReadFile(filepath.Join(a.Root, "go.mod"))
	if err != nil {
		return err
	}
	mod, err := modfile.Parse("go.mod", data, nil)
	if err != nil {
		return err
	}
	var selected module.Version
	for _, require := range mod.Require {
		if require.Mod.Path == "mvdan.cc/sh/v3" {
			selected = require.Mod
			break
		}
	}
	if selected.Path == "" {
		return errors.New("go.mod does not require mvdan.cc/sh/v3")
	}
	client := artifacts.NewClient()
	defer client.Close()
	scratch, err := os.MkdirTemp("", "demi-fork-")
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, os.RemoveAll(scratch)) }()
	upstream, err := moduleUpstream(
		ctx,
		client,
		selected,
		scratch,
		"https://proxy.golang.org",
		sumdb.NewClient(&checksumOps{ctx: ctx, client: client, url: "https://sum.golang.org", key: checksumKey}),
	)
	if err != nil {
		return err
	}
	fork := filepath.Join(a.Root, "third_party/mvdan-sh")
	summary, generated, err := compareFork(ctx, upstream, fork)
	if err != nil {
		return err
	}
	if patch {
		_, err = fmt.Fprint(a.Out, generated)
	} else {
		_, err = fmt.Fprintf(a.Out, "third_party/mvdan-sh %s\n%s", selected.Version, summary)
	}
	if err != nil {
		return err
	}
	recorded, err := os.ReadFile(filepath.Join(fork, "demi.patch"))
	if err != nil {
		return err
	}
	if string(recorded) != generated {
		return errors.New("third_party/mvdan-sh/demi.patch differs from the fork comparison")
	}
	return nil
}

func moduleUpstream(
	ctx context.Context,
	client *artifacts.Client,
	selected module.Version,
	scratch, proxy string,
	checksums *sumdb.Client,
) (string, error) {
	path, err := module.EscapePath(selected.Path)
	if err != nil {
		return "", err
	}
	version, err := module.EscapeVersion(selected.Version)
	if err != nil {
		return "", err
	}
	archive := filepath.Join(scratch, "module.zip")
	file, err := os.Create(archive)
	if err != nil {
		return "", err
	}
	_, downloadErr := artifacts.DownloadMeasured(
		ctx,
		client,
		strings.TrimRight(proxy, "/")+"/"+path+"/@v/"+version+".zip",
		64*1024*1024,
		file,
	)
	if err := errors.Join(downloadErr, file.Close()); err != nil {
		return "", err
	}
	hash, err := dirhash.HashZip(archive, dirhash.Hash1)
	if err != nil {
		return "", err
	}
	lines, err := checksums.Lookup(selected.Path, selected.Version)
	if err != nil {
		return "", err
	}
	if !slices.Contains(lines, selected.Path+" "+selected.Version+" "+hash) {
		return "", fmt.Errorf(
			"%s %s does not match the hash the Go checksum database records",
			selected.Path,
			selected.Version,
		)
	}
	destination := filepath.Join(scratch, "upstream")
	if err := modzip.Unzip(destination, selected, archive); err != nil {
		return "", err
	}
	return destination, nil
}

func forkFiles(ctx context.Context, root string) (map[string][]byte, error) {
	files := make(map[string][]byte)
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if err := ctx.Err(); err != nil {
			return err
		}
		if entry.IsDir() {
			return nil
		}
		relative, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		relative = filepath.ToSlash(relative)
		if relative == "demi.patch" {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		files[relative] = data
		return nil
	})
	return files, err
}

func compareFork(ctx context.Context, upstream, fork string) (string, string, error) {
	theirs, err := forkFiles(ctx, upstream)
	if err != nil {
		return "", "", err
	}
	ours, err := forkFiles(ctx, fork)
	if err != nil {
		return "", "", err
	}
	var names []string
	for name := range theirs {
		names = append(names, name)
	}
	for name := range ours {
		if _, ok := theirs[name]; !ok {
			names = append(names, name)
		}
	}
	slices.Sort(names)
	var summary, patch strings.Builder
	changed, added, removed, leftOut := 0, 0, 0, 0
	for _, name := range names {
		before, had := theirs[name]
		after, has := ours[name]
		if bytes.Equal(before, after) && had == has {
			continue
		}
		// Omitted upstream CI and test fixtures are named separately in the summary.
		if !has && (strings.HasPrefix(name, ".github/") || strings.HasPrefix(name, "testdata/")) {
			leftOut++
			continue
		}
		kind := "changed"
		if !had {
			kind = "added"
			added++
		} else if !has {
			kind = "removed"
			removed++
		} else {
			changed++
		}
		dmp := diffmatchpatch.New()
		dmp.DiffTimeout = 0
		left, right, lines := dmp.DiffLinesToRunes(string(before), string(after))
		differences := dmp.DiffCharsToLines(dmp.DiffMainRunes(left, right, false), lines)
		inserted, deleted := countDiffLines(differences)
		fmt.Fprintf(&summary, "  %s %s (+%d -%d)\n", kind, name, inserted, deleted)
		patch.WriteString(unifiedFile(name, string(before), string(after), differences))
	}
	return fmt.Sprintf(
		"%d changed, %d added, %d removed, %d left out\n%s",
		changed,
		added,
		removed,
		leftOut,
		summary.String(),
	), patch.String(), nil
}

// unifiedFile renders go-diff's line edits in the POSIX unified format. go-diff's
// PatchToText uses character offsets and URL escaping, so it cannot be applied
// by patch. This adapter only groups the library's edits into three-line context
// hunks; go-diff remains the sole owner of deciding which lines changed.
func unifiedFile(name, before, after string, differences []diffmatchpatch.Diff) string {
	var lines []diffLine
	old, next := 1, 1
	for _, difference := range differences {
		for _, text := range strings.SplitAfter(difference.Text, "\n") {
			if text == "" {
				continue
			}
			lines = append(lines, diffLine{difference.Type, text, old, next})
			if difference.Type != diffmatchpatch.DiffInsert {
				old++
			}
			if difference.Type != diffmatchpatch.DiffDelete {
				next++
			}
		}
	}
	var out strings.Builder
	oldName, newName := "a/"+name, "b/"+name
	if before == "" {
		oldName = "/dev/null"
	}
	if after == "" {
		newName = "/dev/null"
	}
	fmt.Fprintf(&out, "--- %s\n+++ %s\n", oldName, newName)
	for cursor := 0; cursor < len(lines); {
		for cursor < len(lines) && lines[cursor].kind == diffmatchpatch.DiffEqual {
			cursor++
		}
		if cursor == len(lines) {
			break
		}
		start := max(0, cursor-3)
		end := cursor + 1
		for scan := cursor + 1; scan < len(lines); scan++ {
			if lines[scan].kind != diffmatchpatch.DiffEqual {
				end = scan + 1
			}
			if scan-end >= 6 {
				break
			}
		}
		end = min(len(lines), end+3)
		writeDiffHunk(&out, lines[start:end])
		cursor = end
	}
	return out.String()
}

func countDiffLines(differences []diffmatchpatch.Diff) (inserted, deleted int) {
	for _, d := range differences {
		count := len(strings.SplitAfter(d.Text, "\n"))
		if strings.HasSuffix(d.Text, "\n") {
			count--
		}
		switch d.Type {
		case diffmatchpatch.DiffInsert:
			inserted += count
		case diffmatchpatch.DiffDelete:
			deleted += count
		case diffmatchpatch.DiffEqual:
		}
	}

	return inserted, deleted
}

type diffLine struct {
	kind      diffmatchpatch.Operation
	text      string
	old, next int
}

func writeDiffHunk(out *strings.Builder, lines []diffLine) {
	oldCount, newCount := 0, 0
	for _, line := range lines {
		if line.kind != diffmatchpatch.DiffInsert {
			oldCount++
		}
		if line.kind != diffmatchpatch.DiffDelete {
			newCount++
		}
	}
	oldStart, newStart := lines[0].old, lines[0].next
	if oldCount == 0 {
		oldStart--
	}
	if newCount == 0 {
		newStart--
	}
	fmt.Fprintf(out, "@@ -%d,%d +%d,%d @@\n", oldStart, oldCount, newStart, newCount)
	for _, line := range lines {
		prefix := " "
		switch line.kind {
		case diffmatchpatch.DiffInsert:
			prefix = "+"
		case diffmatchpatch.DiffDelete:
			prefix = "-"
		case diffmatchpatch.DiffEqual:
		}
		out.WriteString(prefix)
		out.WriteString(line.text)
		if !strings.HasSuffix(line.text, "\n") {
			out.WriteString("\n\\ No newline at end of file\n")
		}
	}
}
