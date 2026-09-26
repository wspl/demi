package definitions

import "github.com/wspl/demi/internal/testing/gnucorpus"

func lsCases() []gnucorpus.Case {
	tree := gnucorpus.Tree{
		Files: map[string][]byte{
			"b.txt":     []byte("bb"),
			"a.txt":     []byte("a"),
			"sub/c.txt": []byte("ccc"),
		},
		Dirs:     []string{"empty-dir"},
		Symlinks: map[string]string{"link.txt": "a.txt"},
	}
	hidden := gnucorpus.Tree{Files: map[string][]byte{".hidden": []byte("h"), "visible": []byte("v")}}
	return []gnucorpus.Case{
		core("default-listing", []string{"."}, "", tree),
		core("one-per-line", []string{"-1", "."}, "", tree),
		core("almost-all-includes-dotfiles", []string{"-A", "."}, "", hidden),
		core("all-includes-dot-and-dotdot", []string{"-a", "."}, "", hidden),
		core("recursive", []string{"-R", "."}, "", tree),
		core("classify", []string{"-F", "."}, "", tree),
		core("directory-itself", []string{"-d", "sub"}, "", tree),
		core("multiple-args", []string{"a.txt", "sub"}, "", tree),
		core("missing-path", []string{"missing"}, "", tree),
		extended("sort-by-size", []string{"-S", "-1", "."}, "", tree),
		extended("reverse", []string{"-r", "-1", "."}, "", tree),
		extended("sort-by-extension", []string{"-X", "-1", "."}, "", tree),
		extended("color-never-explicit", []string{"--color=never", "-1", "."}, "", tree),
	}
}

func cpCases() []gnucorpus.Case {
	tree := files(map[string]string{"src.txt": "content\n"})
	dirTree := gnucorpus.Tree{
		Files: map[string][]byte{"srcdir/a.txt": []byte("a\n"), "srcdir/sub/b.txt": []byte("b\n")},
	}
	destExists := files(map[string]string{"src.txt": "content\n", "dst.txt": "old\n"})
	symlinkTree := gnucorpus.Tree{
		Files:    map[string][]byte{"target.txt": []byte("real\n")},
		Symlinks: map[string]string{"link.txt": "target.txt"},
	}
	return []gnucorpus.Case{
		core("copy-to-new-name", []string{"src.txt", "dst.txt"}, "", tree),
		core("copy-into-directory", []string{"src.txt", "dest-dir/"}, "", gnucorpus.Tree{
			Files: map[string][]byte{"src.txt": []byte("content\n")}, Dirs: []string{"dest-dir"},
		}),
		core("recursive-directory", []string{"-r", "srcdir", "destdir"}, "", dirTree),
		core("overwrites-by-default", []string{"src.txt", "dst.txt"}, "", destExists),
		core("no-clobber-skips-existing", []string{"-n", "src.txt", "dst.txt"}, "", destExists),
		core("missing-source-error", []string{"missing.txt", "dst.txt"}, "", tree),
		core("directory-without-r-errors", []string{"srcdir", "destdir"}, "", dirTree),
		extended("preserve-mode", []string{"-p", "src.txt", "dst.txt"}, "", tree),
		extended("follows-symlink-by-default", []string{"link.txt", "copy.txt"}, "", symlinkTree),
		extended("no-dereference-copies-link", []string{"-P", "link.txt", "copy.txt"}, "", symlinkTree),
		extended("archive-preserves-tree", []string{"-a", "srcdir", "destdir"}, "", dirTree),
	}
}

func mvCases() []gnucorpus.Case {
	tree := files(map[string]string{"src.txt": "content\n"})
	destExists := files(map[string]string{"src.txt": "content\n", "dst.txt": "old\n"})
	dirTree := gnucorpus.Tree{Files: map[string][]byte{"srcdir/a.txt": []byte("a\n")}}
	return []gnucorpus.Case{
		core("rename-file", []string{"src.txt", "dst.txt"}, "", tree),
		core("move-into-directory", []string{"src.txt", "dest-dir/"}, "", gnucorpus.Tree{
			Files: map[string][]byte{"src.txt": []byte("content\n")}, Dirs: []string{"dest-dir"},
		}),
		core("move-directory", []string{"srcdir", "destdir"}, "", dirTree),
		core("overwrites-by-default", []string{"src.txt", "dst.txt"}, "", destExists),
		core("no-clobber-skips-existing", []string{"-n", "src.txt", "dst.txt"}, "", destExists),
		core("missing-source-error", []string{"missing.txt", "dst.txt"}, "", tree),
	}
}

func rmCases() []gnucorpus.Case {
	tree := files(map[string]string{"a.txt": "a\n"})
	dirTree := gnucorpus.Tree{Files: map[string][]byte{"sub/a.txt": []byte("a\n")}}
	return []gnucorpus.Case{
		core("remove-file", []string{"a.txt"}, "", tree),
		core("missing-file-errors", []string{"missing.txt"}, "", tree),
		core("force-ignores-missing", []string{"-f", "missing.txt"}, "", tree),
		core("directory-without-r-errors", []string{"sub"}, "", dirTree),
		core("recursive-removes-directory", []string{"-r", "sub"}, "", dirTree),
		core("recursive-force-combined", []string{"-rf", "sub"}, "", dirTree),
		extended("multiple-operands", []string{"a.txt", "missing.txt"}, "", tree),
	}
}

func mkdirCases() []gnucorpus.Case {
	existing := gnucorpus.Tree{Dirs: []string{"already"}}
	return []gnucorpus.Case{
		core("create-directory", []string{"newdir"}, "", gnucorpus.Tree{}),
		core("already-exists-errors", []string{"already"}, "", existing),
		core("parents-flag-creates-chain", []string{"-p", "a/b/c"}, "", gnucorpus.Tree{}),
		core("parents-flag-ok-if-exists", []string{"-p", "already"}, "", existing),
		core("multiple-operands", []string{"one", "two"}, "", gnucorpus.Tree{}),
		extended("mode-flag", []string{"-m", "700", "restricted"}, "", gnucorpus.Tree{}),
	}
}

func rmdirCases() []gnucorpus.Case {
	empty := gnucorpus.Tree{Dirs: []string{"empty"}}
	nonEmpty := gnucorpus.Tree{Files: map[string][]byte{"full/a.txt": []byte("a\n")}}
	nested := gnucorpus.Tree{Dirs: []string{"a/b/c"}}
	return []gnucorpus.Case{
		core("remove-empty-directory", []string{"empty"}, "", empty),
		core("non-empty-directory-errors", []string{"full"}, "", nonEmpty),
		core("missing-directory-errors", []string{"missing"}, "", gnucorpus.Tree{}),
		extended("parents-removes-chain", []string{"-p", "a/b/c"}, "", nested),
	}
}

func touchCases() []gnucorpus.Case {
	existing := gnucorpus.Tree{Files: map[string][]byte{"existing.txt": []byte("content\n")}}
	withRef := gnucorpus.Tree{
		Files: map[string][]byte{"ref.txt": []byte("r\n")},
		Times: map[string]int64{"ref.txt": 1_700_000_000},
	}

	explicitTimestamp := core("explicit-timestamp", []string{"-t", "202401020304.05", "stamped.txt"}, "", gnucorpus.Tree{})
	explicitTimestamp.CheckTimes = []string{"stamped.txt"}

	referenceFile := core("reference-file", []string{"-r", "ref.txt", "copy-time.txt"}, "", withRef)
	referenceFile.CheckTimes = []string{"copy-time.txt"}

	mtimeOnly := extended("mtime-only", []string{"-m", "-t", "202401020304.05", "mtime-only.txt"}, "", gnucorpus.Tree{})
	mtimeOnly.CheckTimes = []string{"mtime-only.txt"}

	dateString := extended("date-string", []string{"-d", "2024-01-02 03:04:05", "dated.txt"}, "", gnucorpus.Tree{})
	dateString.CheckTimes = []string{"dated.txt"}

	return []gnucorpus.Case{
		core("creates-new-empty-file", []string{"new.txt"}, "", gnucorpus.Tree{}),
		core("leaves-existing-content-unchanged", []string{"existing.txt"}, "", existing),
		explicitTimestamp,
		core("no-create-on-missing", []string{"-c", "missing.txt"}, "", gnucorpus.Tree{}),
		referenceFile,
		mtimeOnly,
		dateString,
	}
}

func statCases() []gnucorpus.Case {
	tree := files(map[string]string{"a.txt": "hello\n"})
	link := gnucorpus.Tree{
		Files:    map[string][]byte{"target.txt": []byte("t\n")},
		Symlinks: map[string]string{"link.txt": "target.txt"},
	}
	return []gnucorpus.Case{
		core("size-and-type", []string{"-c", "%s %F", "a.txt"}, "", tree),
		core("octal-permissions", []string{"-c", "%a", "a.txt"}, "", tree),
		core("numeric-owner", []string{"-c", "%u:%g", "a.txt"}, "", tree),
		core("missing-file-errors", []string{"missing.txt"}, "", tree),
		core("filename-format", []string{"-c", "%n", "a.txt"}, "", tree),
		extended("dereference-symlink", []string{"-L", "-c", "%s %F", "link.txt"}, "", link),
		extended("symlink-itself", []string{"-c", "%F", "link.txt"}, "", link),
	}
}

func duCases() []gnucorpus.Case {
	tree := gnucorpus.Tree{Files: map[string][]byte{
		"a.txt":     make([]byte, 1000),
		"sub/b.txt": make([]byte, 2000),
	}}
	return []gnucorpus.Case{
		core("apparent-size-single-file", []string{"-b", "a.txt"}, "", tree),
		core("apparent-size-summary", []string{"-sb", "."}, "", tree),
		core("apparent-size-all-files", []string{"-ab", "."}, "", tree),
		core("missing-path-errors", []string{"-b", "missing"}, "", tree),
		extended("max-depth", []string{"-b", "--max-depth=1", "."}, "", tree),
	}
}

func dfCases() []gnucorpus.Case {
	// df reports the host's real filesystem sizes and mount points, which
	// this harness's fixed input tree cannot pin down, so its corpus is
	// limited to argument handling that does not depend on live disk state.
	return []gnucorpus.Case{
		core("missing-path-errors", []string{"/does/not/exist"}, "", gnucorpus.Tree{}),
		extended("type-filter-with-no-match", []string{"-t", "no-such-fs-type", "."}, "", gnucorpus.Tree{}),
	}
}

func chmodCases() []gnucorpus.Case {
	tree := gnucorpus.Tree{
		Files: map[string][]byte{"a.txt": []byte("a\n")},
		Modes: map[string]uint32{"a.txt": 0o644},
	}
	dirTree := gnucorpus.Tree{
		Files: map[string][]byte{"sub/a.txt": []byte("a\n")},
		Modes: map[string]uint32{"sub": 0o755, "sub/a.txt": 0o644},
	}
	return []gnucorpus.Case{
		core("octal-mode", []string{"600", "a.txt"}, "", tree),
		core("symbolic-add-execute", []string{"u+x", "a.txt"}, "", tree),
		core("symbolic-remove-write-for-others", []string{"o-w", "a.txt"}, "", tree),
		core("recursive", []string{"-R", "700", "sub"}, "", dirTree),
		core("missing-file-errors", []string{"600", "missing.txt"}, "", tree),
		extended("symbolic-assign-all", []string{"a=r", "a.txt"}, "", tree),
		extended("reference-file", []string{"--reference=a.txt", "b.txt"}, "", gnucorpus.Tree{
			Files: map[string][]byte{"a.txt": []byte("a\n"), "b.txt": []byte("b\n")},
			Modes: map[string]uint32{"a.txt": 0o600},
		}),
	}
}

func chownCases() []gnucorpus.Case {
	tree := files(map[string]string{"a.txt": "a\n"})
	return []gnucorpus.Case{
		core("numeric-owner-and-group", []string{"0:0", "a.txt"}, "", tree),
		core("owner-only", []string{"0", "a.txt"}, "", tree),
		core("group-only", []string{":0", "a.txt"}, "", tree),
		core("missing-file-errors", []string{"0:0", "missing.txt"}, "", tree),
		extended("recursive", []string{"-R", "0:0", "sub"}, "", gnucorpus.Tree{
			Files: map[string][]byte{"sub/a.txt": []byte("a\n")},
		}),
		extended("unknown-user-errors", []string{"no-such-user-xyz", "a.txt"}, "", tree),
	}
}

func realpathCases() []gnucorpus.Case {
	// realpath's plain output is the absolute path of the working directory,
	// which is a fresh, unpredictable temp path for every recording and
	// every check run; --relative-to=. reports the same resolution relative
	// to the working directory instead, which is deterministic. Plain
	// absolute-path output is not part of this corpus; see
	// testdata/gnu/README.md.
	tree := gnucorpus.Tree{
		Files:    map[string][]byte{"dir/target.txt": []byte("t\n")},
		Symlinks: map[string]string{"link.txt": "dir/target.txt"},
	}
	return []gnucorpus.Case{
		core("resolves-relative-path", []string{"--relative-to=.", "dir/target.txt"}, "", tree),
		core("resolves-through-symlink", []string{"--relative-to=.", "link.txt"}, "", tree),
		core("dot-is-working-directory", []string{"--relative-to=.", "."}, "", tree),
		// Plain realpath canonicalizes a path whether or not it exists; -e
		// (--canonicalize-existing) is what makes a missing path an error.
		core("missing-path-with-existing-only-errors", []string{"-e", "missing.txt"}, "", tree),
		extended("relative-to-subdirectory", []string{"--relative-to=dir", "dir/target.txt"}, "", tree),
	}
}

func mktempCases() []gnucorpus.Case {
	// mktemp's whole purpose is an unpredictable name, so a successful run's
	// stdout and the tree entry it creates cannot be pinned down by this
	// exact-comparison harness. Its corpus is limited to deterministic
	// argument-validation errors; see testdata/gnu/README.md.
	return []gnucorpus.Case{
		core("template-without-x-errors", []string{"notemplate"}, "", gnucorpus.Tree{}),
		core("dry-run-flag-usage-error-on-bad-template", []string{"-u", "notemplate"}, "", gnucorpus.Tree{}),
		extended("directory-flag-usage-error-on-bad-template", []string{"-d", "notemplate"}, "", gnucorpus.Tree{}),
	}
}
