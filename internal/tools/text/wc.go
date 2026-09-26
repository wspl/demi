package text

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/tools/text/cli"
)

const wcUsage = `Usage: %[1]s [OPTION]... [FILE]...
  or:  %[1]s [OPTION]... --files0-from=F
Print newline, word, and byte counts for each FILE, and a total line if
more than one FILE is specified.  A word is a nonempty sequence of non white
space delimited by white space characters or by start or end of input.

With no FILE, or when FILE is -, read standard input.

The options below may be used to select which counts are printed, always in
the following order: newline, word, character, byte, maximum line length.
  -c, --bytes            print the byte counts
  -m, --chars            print the character counts
  -l, --lines            print the newline counts
      --files0-from=F    read input from the files specified by
                           NUL-terminated names in file F;
                           If F is - then read names from standard input
  -L, --max-line-length  print the maximum display width
  -w, --words            print the word counts
      --total=WHEN       when to print a line with total counts;
                           WHEN can be: auto, always, only, never
      --help             display this help and exit
      --version          output version information and exit
`

var wcOptions = []cli.Option{
	{Short: 'c', Long: "bytes"},
	{Short: 'm', Long: "chars"},
	{Short: 'l', Long: "lines"},
	{Long: "files0-from", Arg: cli.RequiredArg},
	{Short: 'L', Long: "max-line-length"},
	{Short: 'w', Long: "words"},
	{Long: "total", Arg: cli.RequiredArg},
}

// wcCounts are the counts of one input.
type wcCounts struct {
	lines, words, chars, bytes, maxLength uint64
}

// wcSelection says which counts wc prints.
type wcSelection struct {
	lines, words, chars, bytes, maxLength bool
}

func (s wcSelection) size() int {
	size := 0
	for _, selected := range []bool{s.lines, s.words, s.chars, s.bytes, s.maxLength} {
		if selected {
			size++
		}
	}
	return size
}

// wcTotal says when wc prints the total line.
type wcTotal int

const (
	totalAuto wcTotal = iota
	totalAlways
	totalOnly
	totalNever
)

func wc(inv *toolctx.Invocation, args []string) int {
	prog := &cli.Program{Name: args[0], Inv: inv, Usage: wcUsage, BadUsage: 1}
	options, operands, status, done := prog.Parse(args[1:], wcOptions)
	if done {
		return status
	}
	var selection wcSelection
	total := totalAuto
	filesFrom := ""
	hasFilesFrom := false
	for _, option := range options {
		switch option.Key {
		case "bytes":
			selection.bytes = true
		case "chars":
			selection.chars = true
		case "lines":
			selection.lines = true
		case "max-line-length":
			selection.maxLength = true
		case "words":
			selection.words = true
		case "files0-from":
			filesFrom = option.Value
			hasFilesFrom = true
		case "total":
			when, err := cli.Argmatch("--total", option.Value, []cli.Choice{
				{Key: "auto", Words: []string{"auto"}},
				{Key: "always", Words: []string{"always"}},
				{Key: "only", Words: []string{"only"}},
				{Key: "never", Words: []string{"never"}},
			})
			if err != nil {
				return prog.UsageError(err)
			}
			total = map[string]wcTotal{"auto": totalAuto, "always": totalAlways, "only": totalOnly, "never": totalNever}[when]
		}
	}
	if selection.size() == 0 {
		selection = wcSelection{lines: true, words: true, bytes: true}
	}
	if hasFilesFrom && len(operands) > 0 {
		prog.Errorf("extra operand %s\nfile operands cannot be combined with --files0-from", cli.Quote(operands[0]))
		prog.TryHelp()
		return 1
	}
	run := &wcRun{prog: prog, inv: inv, selection: selection, total: total}
	names := operands
	namesKnown := true
	if hasFilesFrom {
		var err error
		names, namesKnown, err = readFileNames(inv, filesFrom)
		if err != nil {
			prog.Errorf("cannot open %s for reading: %s", cli.Quote(filesFrom), cli.Strerror(err))
			return 1
		}
	} else if len(names) == 0 {
		names = []string{cli.StdinName}
		run.implicitStdin = true
	}
	run.width = run.numberWidth(names, namesKnown)
	return run.run(names, filesFrom)
}

// readFileNames reads the NUL-terminated names of --files0-from. Names
// read from standard input count as not known in advance, as for GNU wc
// reading them from a pipe.
func readFileNames(inv *toolctx.Invocation, from string) (names []string, known bool, err error) {
	input, err := cli.Open(inv, from)
	if err != nil {
		return nil, false, err
	}
	data, readErr := io.ReadAll(input)
	closeErr := input.Close()
	if readErr != nil {
		return nil, false, readErr
	}
	if closeErr != nil {
		return nil, false, closeErr
	}
	text := strings.TrimSuffix(string(data), "\x00")
	if text != "" {
		names = strings.Split(text, "\x00")
	}
	return names, input.File != nil, nil
}

// wcRun is one run of wc.
type wcRun struct {
	prog      *cli.Program
	inv       *toolctx.Invocation
	selection wcSelection
	total     wcTotal
	width     int
	// implicitStdin is set when no operand was given; the counts of
	// standard input are then printed without a name.
	implicitStdin bool
	out           *cli.Output
}

// numberWidth is the column width GNU wc uses: wide enough for the total
// size of the regular files, at least 7 when an input's size is unknown,
// and 1 when a single number is printed per line for one input.
func (r *wcRun) numberWidth(names []string, known bool) int {
	if r.total == totalOnly || !known || (r.selection.size() == 1 && len(names) == 1) {
		return 1
	}
	minimum := 1
	var sum uint64
	for _, name := range names {
		if name == "" {
			continue
		}
		if name == cli.StdinName {
			minimum = 7
			continue
		}
		info, err := r.inv.Files.Stat(name)
		if err != nil {
			continue
		}
		if !info.Mode().IsRegular() {
			minimum = 7
			continue
		}
		sum += uint64(info.Size())
	}
	return max(minimum, len(strconv.FormatUint(sum, 10)))
}

func (r *wcRun) run(names []string, filesFrom string) int {
	r.out = cli.NewOutput(cli.Writer(r.inv.Context, r.inv.Stdout))
	status := 0
	var sum wcCounts
	for index, name := range names {
		if name == "" {
			r.prog.Errorf("%s:%d: invalid zero-length file name", cli.QuoteName(filesFrom), index+1)
			status = 1
			continue
		}
		if filesFrom == cli.StdinName && name == cli.StdinName {
			r.prog.Errorf("when reading file names from stdin, no file name of %s allowed", cli.Quote(name))
			status = 1
			continue
		}
		counts, opened, err := r.count(name)
		if err != nil {
			r.prog.FileError(name, err)
			status = 1
		}
		if !opened {
			continue
		}
		sum.add(counts)
		if r.total != totalOnly {
			if r.implicitStdin {
				name = ""
			}
			r.print(counts, name)
		}
	}
	printTotal := r.total == totalAlways || r.total == totalOnly || (r.total == totalAuto && len(names) > 1)
	if printTotal {
		name := "total"
		if r.total == totalOnly {
			name = ""
		}
		r.print(sum, name)
	}
	if err := r.out.Flush(); err != nil {
		return r.prog.WriteError(err)
	}
	return status
}

func (c *wcCounts) add(other wcCounts) {
	c.lines += other.lines
	c.words += other.words
	c.chars += other.chars
	c.bytes += other.bytes
	c.maxLength = max(c.maxLength, other.maxLength)
}

// print writes one line of counts.
func (r *wcRun) print(counts wcCounts, name string) {
	var fields []string
	for _, field := range []struct {
		selected bool
		value    uint64
	}{
		{r.selection.lines, counts.lines},
		{r.selection.words, counts.words},
		{r.selection.chars, counts.chars},
		{r.selection.bytes, counts.bytes},
		{r.selection.maxLength, counts.maxLength},
	} {
		if field.selected {
			fields = append(fields, fmt.Sprintf("%*d", r.width, field.value))
		}
	}
	line := strings.Join(fields, " ")
	if name != "" {
		line += " " + name
	}
	r.out.PutString(line + "\n")
}

// count counts one input. opened is false when the input could not be
// opened; the counts up to a read error are still printed.
func (r *wcRun) count(name string) (counts wcCounts, opened bool, err error) {
	input, err := cli.Open(r.inv, name)
	if err != nil {
		return counts, false, err
	}
	counter := wcCounter{}
	buffer := make([]byte, 64*1024)
	for {
		read, readErr := input.Read(buffer)
		counter.write(buffer[:read])
		if errors.Is(readErr, io.EOF) {
			break
		}
		if readErr != nil {
			err = readErr
			break
		}
	}
	closeErr := input.Close()
	if err == nil {
		err = closeErr
	}
	return counter.finish(), true, err
}

// wcCounter counts bytes as GNU wc does in the C locale: a word is a run of
// bytes that holds a printable character and no white space; bytes that
// are neither printable nor white space do not start or end a word.
type wcCounter struct {
	counts   wcCounts
	inWord   bool
	position uint64
}

func (c *wcCounter) write(chunk []byte) {
	c.counts.bytes += uint64(len(chunk))
	c.counts.chars += uint64(len(chunk))
	c.counts.lines += uint64(bytes.Count(chunk, []byte{'\n'}))
	for _, b := range chunk {
		switch b {
		case '\n':
			c.endLine()
			c.inWord = false
		case '\r', '\f':
			c.endLine()
			c.inWord = false
		case '\t':
			c.position += 8 - c.position%8
			c.inWord = false
		case ' ', '\v':
			if b == ' ' {
				c.position++
			}
			c.inWord = false
		default:
			if b > ' ' && b < 0x7f {
				c.position++
				if !c.inWord {
					c.counts.words++
					c.inWord = true
				}
			}
		}
	}
}

// endLine ends a line or, for \r and \f, returns to its first column.
func (c *wcCounter) endLine() {
	c.counts.maxLength = max(c.counts.maxLength, c.position)
	c.position = 0
}

func (c *wcCounter) finish() wcCounts {
	c.endLine()
	return c.counts
}
