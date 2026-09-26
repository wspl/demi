package text

import (
	"bytes"
	"crypto/rand"
	"errors"
	"fmt"
	"io"
	"os"
	"sort"
	"strings"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/tools/text/cli"
)

const sortUsage = `Usage: %[1]s [OPTION]... [FILE]...
  or:  %[1]s [OPTION]... --files0-from=F
Write sorted concatenation of all FILE(s) to standard output.

With no FILE, or when FILE is -, read standard input.

Ordering options:

  -b, --ignore-leading-blanks  ignore leading blanks
  -d, --dictionary-order      consider only blanks and alphanumeric characters
  -f, --ignore-case           fold lower case to upper case characters
  -g, --general-numeric-sort  compare according to general numerical value
  -i, --ignore-nonprinting    consider only printable characters
  -M, --month-sort            compare (unknown) < 'JAN' < ... < 'DEC'
  -h, --human-numeric-sort    compare human readable numbers (e.g., 2K 1G)
  -n, --numeric-sort          compare according to string numerical value
  -R, --random-sort           shuffle, but group identical keys
      --random-source=FILE    get random bytes from FILE
  -r, --reverse               reverse the result of comparisons
      --sort=WORD             sort according to WORD:
                                general-numeric -g, human-numeric -h, month -M,
                                numeric -n, random -R, version -V
  -V, --version-sort          natural sort of (version) numbers within text

Other options:

      --batch-size=NMERGE   accepted for compatibility (at least 2)
  -c, --check, --check=diagnose-first  check for sorted input; do not sort
  -C, --check=quiet, --check=silent  like -c, but do not report first bad line
      --compress-program=PROG  accepted for compatibility; no temporary files
                              are written
      --files0-from=F       read input from the files specified by
                            NUL-terminated names in file F;
                            If F is - then read names from standard input
  -k, --key=KEYDEF          sort via a key; KEYDEF gives location and type
  -m, --merge               merge already sorted files; do not sort
  -o, --output=FILE         write result to FILE instead of standard output
  -s, --stable              stabilize sort by disabling last-resort comparison
  -S, --buffer-size=SIZE    accepted for compatibility
  -t, --field-separator=SEP  use SEP instead of non-blank to blank transition
  -T, --temporary-directory=DIR  accepted for compatibility
      --parallel=N          accepted for compatibility
  -u, --unique              with -c, check for strict ordering;
                              without -c, output only the first of an equal run
  -z, --zero-terminated     line delimiter is NUL, not newline
      --help                display this help and exit
      --version             output version information and exit

KEYDEF is F[.C][OPTS][,F[.C][OPTS]] for start and stop position, where F is a
field number and C a character position in the field; both are origin 1, and
the stop position defaults to the line's end.  If neither -t nor -b is in
effect, characters in a field are counted from the beginning of the preceding
whitespace.  OPTS is one or more single-letter ordering options [bdfgiMhnRrV],
which override global ordering options for that key.  If no key is given, use
the entire line as the key.

The whole input is sorted in memory.
`

var sortOptions = []cli.Option{
	{Short: 'b', Long: "ignore-leading-blanks"},
	{Short: 'd', Long: "dictionary-order"},
	{Short: 'f', Long: "ignore-case"},
	{Short: 'g', Long: "general-numeric-sort"},
	{Short: 'i', Long: "ignore-nonprinting"},
	{Short: 'M', Long: "month-sort"},
	{Short: 'h', Long: "human-numeric-sort"},
	{Short: 'n', Long: "numeric-sort"},
	{Short: 'R', Long: "random-sort"},
	{Long: "random-source", Arg: cli.RequiredArg},
	{Short: 'r', Long: "reverse"},
	{Long: "sort", Arg: cli.RequiredArg},
	{Short: 'V', Long: "version-sort"},
	{Long: "batch-size", Arg: cli.RequiredArg},
	{Short: 'c', Key: "check"},
	{Long: "check", Arg: cli.OptionalArg},
	{Short: 'C', Key: "C"},
	{Long: "compress-program", Arg: cli.RequiredArg},
	{Long: "debug"},
	{Long: "files0-from", Arg: cli.RequiredArg},
	{Short: 'k', Long: "key", Arg: cli.RequiredArg},
	{Short: 'm', Long: "merge"},
	{Short: 'o', Long: "output", Arg: cli.RequiredArg},
	{Short: 's', Long: "stable"},
	{Short: 'S', Long: "buffer-size", Arg: cli.RequiredArg},
	{Short: 't', Long: "field-separator", Arg: cli.RequiredArg},
	{Short: 'T', Long: "temporary-directory", Arg: cli.RequiredArg},
	{Long: "parallel", Arg: cli.RequiredArg},
	{Short: 'u', Long: "unique"},
	{Short: 'z', Long: "zero-terminated"},
}

// sortCheck is what sort -c does.
type sortCheck int

const (
	checkNone sortCheck = iota
	checkDiagnose
	checkQuiet
)

// sortStatusFailure is sort's exit status for trouble, as opposed to
// disorder found by -c.
const sortStatusFailure = 2

// sortRun is one run of sort.
type sortRun struct {
	prog         *cli.Program
	inv          *toolctx.Invocation
	config       sortConfig
	check        sortCheck
	merge        bool
	output       string
	hasOutput    bool
	delimiter    byte
	randomSource string
	hasRandom    bool
	filesFrom    string
	hasFilesFrom bool
}

func sortMain(inv *toolctx.Invocation, args []string) int {
	prog := &cli.Program{Name: args[0], Inv: inv, Usage: sortUsage, BadUsage: sortStatusFailure}
	options, operands, status, done := prog.Parse(args[1:], sortOptions)
	if done {
		return status
	}
	run := &sortRun{prog: prog, inv: inv, delimiter: '\n'}
	if err := run.configure(options); err != nil {
		var usage *cli.UsageError
		if errors.As(err, &usage) {
			return prog.UsageError(err)
		}
		prog.Errorf("%s", err)
		return sortStatusFailure
	}
	if run.hasFilesFrom {
		if len(operands) > 0 {
			prog.Errorf("extra operand %s\nfile operands cannot be combined with --files0-from", cli.Quote(operands[0]))
			prog.TryHelp()
			return sortStatusFailure
		}
		names, err := run.readFileNames()
		if err != nil {
			return run.fail(err)
		}
		operands = names
	}
	if len(operands) == 0 {
		operands = []string{cli.StdinName}
	}
	if run.check != checkNone && len(operands) > 1 {
		prog.Errorf("extra operand %s not allowed with -%s", cli.Quote(operands[1]), map[sortCheck]string{checkDiagnose: "c", checkQuiet: "C"}[run.check])
		return sortStatusFailure
	}
	if run.config.needsSalt() {
		if err := run.readSalt(); err != nil {
			return run.fail(err)
		}
	}
	inputs := make([][][]byte, 0, len(operands))
	for _, name := range operands {
		lines, err := run.readLines(name)
		if err != nil {
			return run.fail(err)
		}
		inputs = append(inputs, lines)
	}
	if run.check != checkNone {
		return run.checkSorted(operands[0], inputs[0])
	}
	var lines [][]byte
	if run.merge {
		lines = run.config.mergeInputs(inputs)
	} else {
		for _, input := range inputs {
			lines = append(lines, input...)
		}
		if err := run.config.sortLines(inv, lines); err != nil {
			return 1
		}
	}
	if run.config.unique {
		lines = run.config.uniqueLines(lines)
	}
	return run.write(lines)
}

// sortFailure is an error that sort reports as "MESSAGE" and exits 2 for.
type sortFailure struct {
	message string
}

func (e *sortFailure) Error() string {
	return e.message
}

// fail reports an error that ends sort.
func (r *sortRun) fail(err error) int {
	r.prog.Errorf("%s", err)
	return sortStatusFailure
}

// configure applies the options.
func (r *sortRun) configure(options []cli.Parsed) error {
	var global sortOrdering
	var globalKinds sortOrderingKinds
	var keys []sortKey
	for _, option := range options {
		if letter, ok := sortOrderingLetters[option.Key]; ok {
			global.applyFlag(letter, false)
			if letter == 'b' {
				global.applyFlag(letter, true)
			}
			globalKinds.add(letter)
			continue
		}
		switch option.Key {
		case "sort":
			word, err := cli.Argmatch("--sort", option.Value, []cli.Choice{
				{Key: "g", Words: []string{"general-numeric"}},
				{Key: "h", Words: []string{"human-numeric"}},
				{Key: "M", Words: []string{"month"}},
				{Key: "n", Words: []string{"numeric"}},
				{Key: "R", Words: []string{"random"}},
				{Key: "V", Words: []string{"version"}},
			})
			if err != nil {
				return err
			}
			global.applyFlag(word[0], false)
			globalKinds.add(word[0])
		case "random-source":
			r.randomSource = option.Value
			r.hasRandom = true
		case "batch-size":
			size, err := cli.ParseUint(option.Value, "")
			if err != nil && !errors.Is(err, cli.ErrOverflow) {
				return fmt.Errorf("invalid --batch-size argument %s", cli.Quote(option.Value))
			}
			if size < 2 {
				return fmt.Errorf("invalid --batch-size argument %s\n%s: minimum --batch-size argument is '2'", cli.Quote(option.Value), r.prog.Name)
			}
		case "check", "C":
			mode := checkQuiet
			if option.Key == "check" {
				mode = checkDiagnose
				if option.HasValue {
					word, err := cli.Argmatch("--check", option.Value, []cli.Choice{
						{Key: "quiet", Words: []string{"quiet", "silent"}},
						{Key: "diagnose-first", Words: []string{"diagnose-first"}},
					})
					if err != nil {
						return err
					}
					if word == "quiet" {
						mode = checkQuiet
					}
				}
			}
			if r.check != checkNone && r.check != mode {
				return &sortFailure{"options '-cC' are incompatible"}
			}
			r.check = mode
		case "compress-program":
			// No temporary files are written, so there is nothing to
			// compress.
		case "debug":
			return &sortFailure{"--debug is not supported"}
		case "files0-from":
			r.filesFrom = option.Value
			r.hasFilesFrom = true
		case "key":
			key, err := parseSortKey(option.Value)
			if err != nil {
				return err
			}
			keys = append(keys, key)
		case "merge":
			r.merge = true
		case "output":
			if r.hasOutput && r.output != option.Value {
				return &sortFailure{"multiple output files specified"}
			}
			r.output = option.Value
			r.hasOutput = true
		case "stable":
			r.config.stable = true
		case "buffer-size":
			if err := checkBufferSize(option.Value); err != nil {
				return err
			}
		case "field-separator":
			tab, err := parseTab(option.Value)
			if err != nil {
				return err
			}
			if r.config.hasTab && r.config.tab != tab {
				return &sortFailure{"incompatible tabs"}
			}
			r.config.tab = tab
			r.config.hasTab = true
		case "temporary-directory":
			// The whole input is sorted in memory.
		case "parallel":
			count, err := cli.ParseUint(option.Value, "")
			if err != nil && !errors.Is(err, cli.ErrOverflow) {
				return fmt.Errorf("invalid --parallel argument %s", cli.Quote(option.Value))
			}
			if count == 0 {
				return &sortFailure{"number in parallel must be nonzero"}
			}
		case "unique":
			r.config.unique = true
		case "zero-terminated":
			r.delimiter = 0
		}
	}
	if r.check != checkNone && r.hasOutput {
		return &sortFailure{fmt.Sprintf("options '-%co' are incompatible", map[sortCheck]byte{checkDiagnose: 'c', checkQuiet: 'C'}[r.check])}
	}
	for index := range keys {
		if !keys[index].ordering.set() {
			keys[index].ordering = global
			keys[index].kinds = globalKinds
		}
	}
	if len(keys) == 0 {
		keys = []sortKey{{ordering: global, kinds: globalKinds}}
	}
	for _, key := range append([]sortKey{{ordering: global, kinds: globalKinds}}, keys...) {
		if letters := incompatible(key.ordering, key.kinds); letters != "" {
			return &sortFailure{fmt.Sprintf("options '-%s' are incompatible", letters)}
		}
	}
	r.config.keys = keys
	r.config.global = global
	return nil
}

// sortOrderingLetters are the options that are ordering letters.
var sortOrderingLetters = map[string]byte{
	"ignore-leading-blanks": 'b',
	"dictionary-order":      'd',
	"ignore-case":           'f',
	"general-numeric-sort":  'g',
	"ignore-nonprinting":    'i',
	"month-sort":            'M',
	"human-numeric-sort":    'h',
	"numeric-sort":          'n',
	"random-sort":           'R',
	"reverse":               'r',
	"version-sort":          'V',
}

// checkBufferSize validates -S SIZE: a number with an optional unit
// letter or %.
func checkBufferSize(value string) error {
	digits := strings.TrimLeft(value, "0123456789")
	number := value[:len(value)-len(digits)]
	if number == "" {
		return fmt.Errorf("invalid -S argument %s", cli.Quote(value))
	}
	if digits == "" || digits == "%" {
		return nil
	}
	if _, err := cli.ParseUint(value, "bKkMGTPEZYRQ"); err != nil && !errors.Is(err, cli.ErrOverflow) {
		return fmt.Errorf("invalid suffix in -S argument %s", cli.Quote(value))
	}
	return nil
}

// parseTab parses the argument of -t.
func parseTab(value string) (byte, error) {
	switch {
	case value == "":
		return 0, &sortFailure{"empty tab"}
	case value == `\0`:
		return 0, nil
	case len(value) > 1:
		return 0, &sortFailure{fmt.Sprintf("multi-character tab %s", cli.Quote(value))}
	}
	return value[0], nil
}

func (c *sortConfig) needsSalt() bool {
	for _, key := range c.keys {
		if key.ordering.kind == sortRandom {
			return true
		}
	}
	return false
}

// readSalt reads the random salt of -R, from --random-source when given.
func (r *sortRun) readSalt() error {
	salt := make([]byte, 16)
	if !r.hasRandom {
		if _, err := rand.Read(salt); err != nil {
			return err
		}
		r.config.salt = salt
		return nil
	}
	input, err := cli.Open(r.inv, r.randomSource)
	if err != nil {
		return &sortFailure{fmt.Sprintf("open failed: %s: %s", cli.QuoteName(r.randomSource), cli.Strerror(err))}
	}
	_, readErr := io.ReadFull(input, salt)
	closeErr := input.Close()
	if errors.Is(readErr, io.ErrUnexpectedEOF) || errors.Is(readErr, io.EOF) {
		return &sortFailure{fmt.Sprintf("%s: end of file", cli.QuoteName(r.randomSource))}
	}
	if readErr == nil {
		readErr = closeErr
	}
	if readErr != nil {
		return &sortFailure{fmt.Sprintf("%s: %s", cli.QuoteName(r.randomSource), cli.Strerror(readErr))}
	}
	r.config.salt = salt
	return nil
}

// readFileNames reads the NUL-terminated names of --files0-from.
func (r *sortRun) readFileNames() ([]string, error) {
	input, err := cli.Open(r.inv, r.filesFrom)
	if err != nil {
		return nil, &sortFailure{fmt.Sprintf("open failed: %s: %s", cli.QuoteName(r.filesFrom), cli.Strerror(err))}
	}
	data, readErr := io.ReadAll(input)
	closeErr := input.Close()
	if readErr == nil {
		readErr = closeErr
	}
	if readErr != nil {
		return nil, &sortFailure{fmt.Sprintf("read failed: %s: %s", cli.QuoteName(r.filesFrom), cli.Strerror(readErr))}
	}
	if len(data) == 0 {
		return nil, &sortFailure{fmt.Sprintf("no input from %s", cli.Quote(r.filesFrom))}
	}
	names := strings.Split(strings.TrimSuffix(string(data), "\x00"), "\x00")
	for index, name := range names {
		if name == "" {
			return nil, &sortFailure{fmt.Sprintf("%s:%d: invalid zero-length file name", cli.QuoteName(r.filesFrom), index+1)}
		}
		if r.filesFrom == cli.StdinName && name == cli.StdinName {
			return nil, &sortFailure{fmt.Sprintf("when reading file names from stdin, no file name of %s allowed", cli.Quote(name))}
		}
	}
	return names, nil
}

// readLines reads one input and splits it into lines without their
// delimiters.
func (r *sortRun) readLines(name string) ([][]byte, error) {
	input, err := cli.Open(r.inv, name)
	if err != nil {
		return nil, &sortFailure{fmt.Sprintf("cannot read: %s: %s", cli.QuoteName(name), cli.Strerror(err))}
	}
	data, readErr := io.ReadAll(input)
	closeErr := input.Close()
	if readErr == nil {
		readErr = closeErr
	}
	if readErr != nil {
		return nil, &sortFailure{fmt.Sprintf("read failed: %s: %s", cli.QuoteName(name), cli.Strerror(readErr))}
	}
	if len(data) == 0 {
		return nil, nil
	}
	data = bytes.TrimSuffix(data, []byte{r.delimiter})
	return bytes.Split(data, []byte{r.delimiter}), nil
}

// checkSorted implements -c and -C.
func (r *sortRun) checkSorted(name string, lines [][]byte) int {
	for index := 1; index < len(lines); index++ {
		if index%4096 == 0 && r.inv.Context.Err() != nil {
			return 1
		}
		diff := r.config.compare(lines[index-1], lines[index])
		if diff > 0 || (r.config.unique && diff == 0) {
			if r.check == checkDiagnose {
				r.prog.Errorf("%s:%d: disorder: %s", name, index+1, lines[index])
			}
			return 1
		}
	}
	return 0
}

// sortLines sorts in place. It fails only when the job is cancelled.
func (c *sortConfig) sortLines(inv *toolctx.Invocation, lines [][]byte) error {
	comparisons := 0
	var cancelled error
	sort.SliceStable(lines, func(a, b int) bool {
		comparisons++
		if comparisons%65536 == 0 && cancelled == nil {
			cancelled = inv.Context.Err()
		}
		if cancelled != nil {
			return false
		}
		return c.compare(lines[a], lines[b]) < 0
	})
	return cancelled
}

// mergeInputs merges inputs that are each already sorted; on ties the
// earlier input comes first.
func (c *sortConfig) mergeInputs(inputs [][][]byte) [][]byte {
	var merged [][]byte
	positions := make([]int, len(inputs))
	for {
		best := -1
		for index, input := range inputs {
			if positions[index] == len(input) {
				continue
			}
			if best < 0 || c.compare(input[positions[index]], inputs[best][positions[best]]) < 0 {
				best = index
			}
		}
		if best < 0 {
			return merged
		}
		merged = append(merged, inputs[best][positions[best]])
		positions[best]++
	}
}

// uniqueLines keeps the first line of each run of lines with equal keys.
func (c *sortConfig) uniqueLines(lines [][]byte) [][]byte {
	var kept [][]byte
	for _, line := range lines {
		if len(kept) > 0 && c.compareKeys(kept[len(kept)-1], line) == 0 {
			continue
		}
		kept = append(kept, line)
	}
	return kept
}

// write writes the lines to standard output or to the -o file, which is
// opened only now so that it may also be an input.
func (r *sortRun) write(lines [][]byte) int {
	output := cli.Writer(r.inv.Context, r.inv.Stdout)
	var file toolctx.File
	if r.hasOutput {
		var err error
		file, err = r.inv.Files.OpenFile(r.output, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o666&^r.inv.Umask)
		if err != nil {
			r.prog.Errorf("open failed: %s: %s", cli.QuoteName(r.output), cli.Strerror(err))
			return sortStatusFailure
		}
		output = cli.Writer(r.inv.Context, file)
	}
	out := cli.NewOutput(output)
	for _, line := range lines {
		out.Put(line)
		out.PutByte(r.delimiter)
		if out.Err() != nil {
			break
		}
	}
	writeErr := out.Flush()
	name := "standard output"
	if file != nil {
		name = r.output
		closeErr := file.Close()
		if writeErr == nil {
			writeErr = closeErr
		}
	}
	switch {
	case writeErr == nil:
		return 0
	case r.inv.Context.Err() != nil:
		return 1
	case file == nil && cli.IsBrokenPipe(writeErr):
		return cli.StatusBrokenPipe
	}
	r.prog.Errorf("write failed: %s: %s", cli.QuoteName(name), cli.Strerror(writeErr))
	return sortStatusFailure
}
