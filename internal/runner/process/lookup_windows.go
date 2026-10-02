package process

import (
	"os"
	"path/filepath"
	"strings"
)

// lookupExecutable resolves a raw process using its requested environment.
// Windows executable extensions are part of that environment as well.
func lookupExecutable(command, cwd string, env map[string]string) (string, error) {
	value := func(key string) string {
		selected := ""
		for name := range env {
			if strings.EqualFold(name, key) && name > selected {
				selected = name
			}
		}
		return env[selected]
	}
	extensions := value("PATHEXT")
	if extensions == "" {
		extensions = ".COM;.EXE;.BAT;.CMD"
	}
	suffixes := []string{""}
	if filepath.Ext(command) == "" {
		suffixes = strings.Split(extensions, ";")
	}
	directories := append([]string{cwd}, filepath.SplitList(value("PATH"))...)
	if strings.ContainsAny(command, `/\:`) {
		directories = []string{""}
	}
	for _, directory := range directories {
		candidate := filepath.Join(directory, command)
		if !filepath.IsAbs(candidate) {
			candidate = filepath.Join(cwd, candidate)
		}
		for _, suffix := range suffixes {
			info, err := os.Stat(candidate + suffix)
			if err == nil && !info.IsDir() {
				return candidate + suffix, nil
			}
		}
	}
	return "", &os.PathError{Op: "exec", Path: command, Err: os.ErrNotExist}
}
