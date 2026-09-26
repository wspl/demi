package text

import (
	"bufio"
	"bytes"
	"errors"
	"fmt"
	"io"
	"math"
	"os"
	"strings"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/tools/text/cli"
)

const uniqUsage = `Usage: %[1]s [OPTION]... [INPUT [OUTPUT]]
Filter adjacent matching lines from INPUT (or standard input),
writing to OUTPUT (or standard output).

With no options, matching lines are merged to the first occurrence.

  -c, --count           prefix lines by the number of occurrences
  -d, --repeated        only print duplicate lines, one for each group
  -D                    print all duplicate lines
      --all-repeated[=METHOD]  like -D, but allow separating groups
                                 with an empty line;
                                 METHOD={none(default),prepend,separate}
  -f, --skip-fields=N   avoid comparing the first N fields
      --group[=METHOD]  show all items, separating groups with an empty line;
                          METHOD={separate(default),prepend,append,both}
  -i, --ignore-case     ignore differences in case when comparing
  -s, --skip-chars=N    avoid comparing the first N characters
  -u, --unique          only print unique lines
  -z, --zero-terminated     line delimiter is NUL, not newline
  -w, --check-chars=N   compare no more than N characters in lines
      --help            display this help and exit
      --version         output version information and exit

A field is a run of blanks (usually spaces and/or TABs), then non-blank
characters.  Fields are skipped before chars.
`

var uniqOptions = []cli.Option{
	{Short: 'c', Long: "count"},
	{Short: 'd', Long: "repeated"},
	{Short: 'D', Key: "D"},
	{Long: "all-repeated", Arg: cli.OptionalArg},
	{Short: 'f', Long: "skip-fields", Arg: cli.RequiredArg},
	{Long: "group", Arg: cli.OptionalArg},
	{Short: 'i', Long: "ignore-case"},
	{Short: 's', Long: "skip-chars", Arg: cli.RequiredArg},
	{Short: 'u', Long: "unique"},
	{Short: 'z', Long: "zero-terminated"},
	{Short: 'w', Long: "check-chars", Arg: cli.RequiredArg},
}

func init() {
	// -NUM is the obsolete form of -f NUM.
	for digit := '0'; digit <= '9'; digit++ {
		uniqOptions = append(uniqOptions, cli.Option{Short: digit, Key: "digit"})
	}
}

// uniqSeparation says where uniq prints empty lines between groups.
type uniqSeparation int

const (
	separateNone uniqSeparation = iota
	separatePrepend
	separateBetween
	separateAppend
	separateBoth
)

// uniqConfig is a parsed uniq command line.
type uniqConfig struct {
	count      bool
	repeated   bool
	unique     bool
	allRepeat  bool
	group      bool
	separation uniqSeparation
	skipFields uint64
	skipChars  uint64
	checkChars uint64
	ignoreCase bool
	delimiter  byte
}

func uniq(inv *toolctx.Invocation, args []string) int {
	prog := &cli.Program{Name: args[0], Inv: inv, Usage: uniqUsage, BadUsage: 1}
	parsed, status, done := prog.ParseInOrder(args[1:], uniqOptions)
	if done {
		return status
	}
	config := uniqConfig{checkChars: math.MaxUint64, delimiter: '\n'}
	var files []string
	lastDigit := -1
	for _, item := range parsed {
		if item.Key == "digit" {
			digit := uint64(item.Name[1] - '0')
			if lastDigit != item.Index {
				config.skipFields = 0
			}
			config.skipFields = saturatingDecimal(config.skipFields, digit)
			lastDigit = item.Index
			continue
		}
		lastDigit = -1
		if item.Key == cli.Operand {
			if !item.Literal && isObsoleteSkipChars(item.Value) {
				config.skipChars = parseSize(item.Value[1:])
				continue
			}
			if len(files) == 2 {
				prog.Errorf("extra operand %s", cli.Quote(item.Value))
				prog.TryHelp()
				return 1
			}
			files = append(files, item.Value)
			continue
		}
		if err := config.apply(item); err != nil {
			var usage *cli.UsageError
			if errors.As(err, &usage) {
				return prog.UsageError(err)
			}
			prog.Errorf("%s", err)
			return 1
		}
	}
	if config.group && (config.count || config.repeated || config.allRepeat || config.unique) {
		prog.Errorf("--group is mutually exclusive with -c/-d/-D/-u")
		prog.TryHelp()
		return 1
	}
	if config.count && config.allRepeat {
		prog.Errorf("printing all duplicated lines and repeat counts is meaningless")
		prog.TryHelp()
		return 1
	}
	input := cli.StdinName
	if len(files) > 0 {
		input = files[0]
	}
	in, err := cli.Open(inv, input)
	if err != nil {
		prog.FileError(input, err)
		return 1
	}
	defer in.Release()
	output := cli.Writer(inv.Context, inv.Stdout)
	var outputFile toolctx.File
	if len(files) == 2 && files[1] != cli.StdinName {
		outputFile, err = inv.Files.OpenFile(files[1], os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o666&^inv.Umask)
		if err != nil {
			prog.FileError(files[1], err)
			return 1
		}
		output = outputFile
	}
	readErr, writeErr := config.filter(in, output)
	if outputFile != nil {
		if err := outputFile.Close(); err != nil && writeErr == nil {
			writeErr = err
		}
	}
	if readErr != nil {
		prog.FileError(input, readErr)
		return 1
	}
	if writeErr != nil {
		if outputFile != nil {
			prog.FileError(files[1], writeErr)
			return 1
		}
		return prog.WriteError(writeErr)
	}
	return 0
}

// isObsoleteSkipChars reports whether an operand is +NUM, the obsolete
// form of -s NUM.
func isObsoleteSkipChars(operand string) bool {
	if len(operand) < 2 || operand[0] != '+' {
		return false
	}
	return strings.Trim(operand[1:], "0123456789") == ""
}

// saturatingDecimal appends a digit to a number, stopping at the largest
// value.
func saturatingDecimal(number, digit uint64) uint64 {
	if number > (math.MaxUint64-digit)/10 {
		return math.MaxUint64
	}
	return number*10 + digit
}

// parseSize parses a digit string that cannot be invalid, saturating.
func parseSize(digits string) uint64 {
	var number uint64
	for _, digit := range digits {
		number = saturatingDecimal(number, uint64(digit-'0'))
	}
	return number
}

// apply applies one option.
func (c *uniqConfig) apply(option cli.Parsed) error {
	switch option.Key {
	case "count":
		c.count = true
	case "repeated":
		c.repeated = true
	case "D":
		c.allRepeat = true
		c.separation = separateNone
	case "all-repeated":
		c.allRepeat = true
		c.separation = separateNone
		if option.HasValue {
			method, err := cli.Argmatch("--all-repeated", option.Value, []cli.Choice{
				{Key: "none", Words: []string{"none"}},
				{Key: "prepend", Words: []string{"prepend"}},
				{Key: "separate", Words: []string{"separate"}},
			})
			if err != nil {
				return err
			}
			c.separation = map[string]uniqSeparation{"none": separateNone, "prepend": separatePrepend, "separate": separateBetween}[method]
		}
	case "group":
		c.group = true
		c.separation = separateBetween
		if option.HasValue {
			method, err := cli.Argmatch("--group", option.Value, []cli.Choice{
				{Key: "prepend", Words: []string{"prepend"}},
				{Key: "append", Words: []string{"append"}},
				{Key: "separate", Words: []string{"separate"}},
				{Key: "both", Words: []string{"both"}},
			})
			if err != nil {
				return err
			}
			c.separation = map[string]uniqSeparation{"prepend": separatePrepend, "append": separateAppend, "separate": separateBetween, "both": separateBoth}[method]
		}
	case "skip-fields", "skip-chars", "check-chars":
		value, err := cli.ParseUint(option.Value, "")
		if err != nil && !errors.Is(err, cli.ErrOverflow) {
			what := map[string]string{
				"skip-fields": "invalid number of fields to skip",
				"skip-chars":  "invalid number of bytes to skip",
				"check-chars": "invalid number of bytes to compare",
			}[option.Key]
			return fmt.Errorf("%s: %s", option.Value, what)
		}
		switch option.Key {
		case "skip-fields":
			c.skipFields = value
		case "skip-chars":
			c.skipChars = value
		default:
			c.checkChars = value
		}
	case "ignore-case":
		c.ignoreCase = true
	case "unique":
		c.unique = true
	case "zero-terminated":
		c.delimiter = 0
	}
	return nil
}

// key returns the part of a line (without its delimiter) that uniq compares.
func (c *uniqConfig) key(line []byte) []byte {
	position := 0
	for field := uint64(0); field < c.skipFields && position < len(line); field++ {
		for position < len(line) && isBlank(line[position]) {
			position++
		}
		for position < len(line) && !isBlank(line[position]) {
			position++
		}
	}
	line = line[position:]
	if c.skipChars >= uint64(len(line)) {
		return nil
	}
	line = line[c.skipChars:]
	if c.checkChars < uint64(len(line)) {
		line = line[:c.checkChars]
	}
	return line
}

func isBlank(c byte) bool {
	return c == ' ' || c == '\t'
}

func (c *uniqConfig) same(a, b []byte) bool {
	keyA := c.key(a)
	keyB := c.key(b)
	if c.ignoreCase {
		return bytes.EqualFold(keyA, keyB)
	}
	return bytes.Equal(keyA, keyB)
}

// filter reads lines and writes what the options select, group by group.
func (c *uniqConfig) filter(input io.Reader, output io.Writer) (readErr, writeErr error) {
	reader := bufio.NewReaderSize(input, 64*1024)
	w := &uniqWriter{config: c, out: cli.NewOutput(output)}
	var group [][]byte
	for {
		line, err := reader.ReadBytes(c.delimiter)
		if err != nil && err != io.EOF {
			return err, nil
		}
		if len(line) > 0 {
			line = bytes.TrimSuffix(line, []byte{c.delimiter})
			if len(group) > 0 && !c.same(group[0], line) {
				w.emit(group)
				group = group[:0]
			}
			group = append(group, line)
		}
		if err == io.EOF {
			break
		}
		if err := w.out.Err(); err != nil {
			return nil, err
		}
	}
	if len(group) > 0 {
		w.emit(group)
	}
	if c.group && w.groups > 0 && (c.separation == separateAppend || c.separation == separateBoth) {
		w.out.PutByte(c.delimiter)
	}
	return nil, w.out.Flush()
}

// uniqWriter writes groups of equal lines.
type uniqWriter struct {
	config *uniqConfig
	out    *cli.Output
	// groups counts the groups written so far.
	groups int
}

func (w *uniqWriter) line(line []byte) {
	w.out.Put(line)
	w.out.PutByte(w.config.delimiter)
}

// emit writes one group of equal lines as the options select.
func (w *uniqWriter) emit(group [][]byte) {
	c := w.config
	switch {
	case c.group:
		if w.groups > 0 || c.separation == separatePrepend || c.separation == separateBoth {
			w.out.PutByte(c.delimiter)
		}
		for _, line := range group {
			w.line(line)
		}
	case c.allRepeat:
		if len(group) < 2 {
			return
		}
		if c.separation == separatePrepend || (c.separation == separateBetween && w.groups > 0) {
			w.out.PutByte(c.delimiter)
		}
		for _, line := range group {
			w.line(line)
		}
	default:
		if c.repeated && len(group) < 2 || c.unique && len(group) > 1 {
			return
		}
		if c.count {
			w.out.Putf("%7d ", len(group))
		}
		w.line(group[0])
	}
	w.groups++
}
