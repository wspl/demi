package wiregen

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
)

// Write generates the Go files of the wire declarations of the package in dir,
// writes them, and removes the generated files that the package no longer
// needs. With a tsPath it also writes the TypeScript schemas there.
func Write(dir, tsPath string) error {
	pkg, err := Load(dir)
	if err != nil {
		return err
	}
	files, err := pkg.GenerateGo()
	if err != nil {
		return err
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		name := entry.Name()
		if _, kept := files[name]; kept || !strings.HasSuffix(name, generatedSuffix) {
			continue
		}
		path := filepath.Join(dir, name)
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if !bytes.HasPrefix(data, []byte(generatedHeader)) {
			continue
		}
		if err := os.Remove(path); err != nil {
			return err
		}
	}
	for name, data := range files {
		if err := os.WriteFile(filepath.Join(dir, name), data, 0o644); err != nil {
			return err
		}
	}
	if tsPath == "" {
		return nil
	}
	source, err := pkg.GenerateTypeScript()
	if err != nil {
		return err
	}
	return os.WriteFile(tsPath, source, 0o644)
}
