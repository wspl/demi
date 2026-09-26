package text

import (
	"io"
	"os"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/tools/text/cli"
)

const teeUsage = `Usage: %[1]s [OPTION]... [FILE]...
Copy standard input to each FILE, and also to standard output.

  -a, --append              append to the given FILEs, do not overwrite
  -i, --ignore-interrupts   ignore interrupt signals
  -p                        operate in a more appropriate MODE with pipes.
      --output-error[=MODE]   set behavior on write error.  See MODE below
      --help                display this help and exit
      --version             output version information and exit

MODE determines behavior with write errors on the outputs:
  warn           diagnose errors writing to any output
  warn-nopipe    diagnose errors writing to any output not a pipe
  exit           exit on error writing to any output
  exit-nopipe    exit on error writing to any output not a pipe
The default MODE for the -p option is 'warn-nopipe'.
With "nopipe" MODEs, exit immediately if all outputs become broken pipes.
The default operation when --output-error is not specified, is to
exit immediately on error writing to a pipe, and diagnose errors
writing to non pipe outputs.
`

var teeOptions = []cli.Option{
	{Short: 'a', Long: "append"},
	{Short: 'i', Long: "ignore-interrupts"},
	{Short: 'p', Key: "p"},
	{Long: "output-error", Arg: cli.OptionalArg},
}

// teeMode is tee's reaction to a failed write.
type teeMode int

const (
	// teeDefault ends tee on a broken pipe, as SIGPIPE does, and diagnoses
	// other errors.
	teeDefault teeMode = iota
	teeWarn
	teeWarnNoPipe
	teeExit
	teeExitNoPipe
)

// teeOutput is one destination of tee.
type teeOutput struct {
	name string
	w    io.Writer
	// file is set for a FILE operand.
	file toolctx.File
}

func tee(inv *toolctx.Invocation, args []string) int {
	prog := &cli.Program{Name: args[0], Inv: inv, Usage: teeUsage, BadUsage: 1}
	options, operands, status, done := prog.Parse(args[1:], teeOptions)
	if done {
		return status
	}
	flag := os.O_WRONLY | os.O_CREATE | os.O_TRUNC
	mode := teeDefault
	for _, option := range options {
		switch option.Key {
		case "append":
			flag = os.O_WRONLY | os.O_CREATE | os.O_APPEND
		case "p":
			mode = teeWarnNoPipe
		case "output-error":
			mode = teeWarnNoPipe
			if option.HasValue {
				chosen, err := cli.Argmatch("--output-error", option.Value, []cli.Choice{
					{Key: "warn", Words: []string{"warn"}},
					{Key: "warn-nopipe", Words: []string{"warn-nopipe"}},
					{Key: "exit", Words: []string{"exit"}},
					{Key: "exit-nopipe", Words: []string{"exit-nopipe"}},
				})
				if err != nil {
					return prog.UsageError(err)
				}
				mode = map[string]teeMode{"warn": teeWarn, "warn-nopipe": teeWarnNoPipe, "exit": teeExit, "exit-nopipe": teeExitNoPipe}[chosen]
			}
		}
	}
	outputs := []*teeOutput{{name: "standard output", w: cli.Writer(inv.Context, inv.Stdout)}}
	status = 0
	for _, name := range operands {
		file, err := inv.Files.OpenFile(name, flag, 0o666&^inv.Umask)
		if err != nil {
			prog.FileError(name, err)
			status = 1
			continue
		}
		outputs = append(outputs, &teeOutput{name: name, w: file, file: file})
	}
	run := &teeRun{prog: prog, mode: mode, outputs: outputs, status: status}
	return run.copy(cli.Reader(inv.Context, inv.Stdin))
}

// teeRun copies standard input to the outputs.
type teeRun struct {
	prog *cli.Program
	mode teeMode
	// outputs are the open outputs; a failed one becomes nil.
	outputs []*teeOutput
	status  int
}

// copy copies input to every output until input ends or no output is left,
// then closes the files.
func (r *teeRun) copy(input io.Reader) int {
	code, stopped := r.pump(input)
	r.closeAll()
	if stopped {
		return code
	}
	return r.status
}

// pump copies until input ends or no output is left. stopped is set when
// tee must end with code at once.
func (r *teeRun) pump(input io.Reader) (code int, stopped bool) {
	buffer := make([]byte, 64*1024)
	for r.active() {
		read, err := input.Read(buffer)
		if read > 0 {
			if code, stop := r.write(buffer[:read]); stop {
				return code, true
			}
		}
		if err == io.EOF {
			return 0, false
		}
		if err != nil {
			if r.prog.Inv.Context.Err() != nil {
				return 1, true
			}
			r.prog.FileError("standard input", err)
			r.status = 1
			return 0, false
		}
	}
	return 0, false
}

// closeAll closes the files still open and reports failed closes.
func (r *teeRun) closeAll() {
	for index, output := range r.outputs {
		if output == nil || output.file == nil {
			continue
		}
		r.outputs[index] = nil
		if err := output.file.Close(); err != nil {
			r.prog.FileError(output.name, err)
			r.status = 1
		}
	}
}

func (r *teeRun) active() bool {
	for _, output := range r.outputs {
		if output != nil {
			return true
		}
	}
	return false
}

// write writes one chunk to every output. stop is set when tee must end
// with code.
func (r *teeRun) write(chunk []byte) (code int, stop bool) {
	for index, output := range r.outputs {
		if output == nil {
			continue
		}
		_, err := output.w.Write(chunk)
		if err == nil {
			continue
		}
		if r.prog.Inv.Context.Err() != nil {
			return 1, true
		}
		brokenPipe := cli.IsBrokenPipe(err)
		switch r.mode {
		case teeDefault:
			if brokenPipe {
				return cli.StatusBrokenPipe, true
			}
			r.fail(index, err)
		case teeWarn:
			r.fail(index, err)
		case teeWarnNoPipe, teeExitNoPipe:
			if brokenPipe {
				r.drop(index)
				continue
			}
			r.fail(index, err)
			if r.mode == teeExitNoPipe {
				return 1, true
			}
		case teeExit:
			r.fail(index, err)
			return 1, true
		}
	}
	return 0, false
}

// fail diagnoses a failed output and stops writing to it.
func (r *teeRun) fail(index int, err error) {
	r.prog.FileError(r.outputs[index].name, err)
	r.status = 1
	r.drop(index)
}

// drop stops writing to an output.
func (r *teeRun) drop(index int) {
	output := r.outputs[index]
	r.outputs[index] = nil
	if output.file != nil {
		// The output already failed or was reported; closing only
		// releases it.
		_ = output.file.Close()
	}
}
