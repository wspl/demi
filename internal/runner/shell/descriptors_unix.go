//go:build darwin || linux

package shell

import (
	"fmt"
	"io"
	"os"
	"os/exec"
	"strconv"
)

// extraDescriptors preserves numbered redirections inherited by external commands.
func extraDescriptors(cmd *exec.Cmd, files map[string]io.ReadWriteCloser) error {
	for name, file := range files {
		n, err := strconv.Atoi(name)
		if err != nil || n < 3 {
			continue
		}
		f, ok := file.(*os.File)
		if tracked, yes := file.(*recordedFile); yes {
			f = tracked.file
			ok = true
		}
		if !ok {
			return fmt.Errorf("descriptor %s has no native file", name)
		}
		for len(cmd.ExtraFiles) <= n-3 {
			cmd.ExtraFiles = append(cmd.ExtraFiles, nil)
		}
		cmd.ExtraFiles[n-3] = f
	}
	return nil
}
