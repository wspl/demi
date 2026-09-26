package text

import (
	"fmt"
	"io"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/tools/text/cli"
)

const trUsage = `Usage: %[1]s [OPTION]... STRING1 [STRING2]
Translate, squeeze, and/or delete characters from standard input,
writing to standard output.  STRING1 and STRING2 specify arrays of
characters ARRAY1 and ARRAY2 that control the action.

  -c, -C, --complement    use the complement of ARRAY1
  -d, --delete            delete characters in ARRAY1, do not translate
  -s, --squeeze-repeats   replace each sequence of a repeated character
                            that is listed in the last specified ARRAY,
                            with a single occurrence of that character
  -t, --truncate-set1     first truncate ARRAY1 to length of ARRAY2
      --help              display this help and exit
      --version           output version information and exit

ARRAYs are specified as strings of characters.  Most represent themselves.
Interpreted sequences are:

  \NNN            character with octal value NNN (1 to 3 octal digits)
  \\              backslash
  \a              audible BEL
  \b              backspace
  \f              form feed
  \n              new line
  \r              return
  \t              horizontal tab
  \v              vertical tab
  CHAR1-CHAR2     all characters from CHAR1 to CHAR2 in ascending order
  [CHAR*]         in ARRAY2, copies of CHAR until length of ARRAY1
  [CHAR*REPEAT]   REPEAT copies of CHAR, REPEAT octal if starting with 0
  [:alnum:]       all letters and digits
  [:alpha:]       all letters
  [:blank:]       all horizontal whitespace
  [:cntrl:]       all control characters
  [:digit:]       all digits
  [:graph:]       all printable characters, not including space
  [:lower:]       all lower case letters
  [:print:]       all printable characters, including space
  [:punct:]       all punctuation characters
  [:space:]       all horizontal or vertical whitespace
  [:upper:]       all upper case letters
  [:xdigit:]      all hexadecimal digits
  [=CHAR=]        all characters which are equivalent to CHAR

Translation occurs if -d is not given and both STRING1 and STRING2 appear.
-t is only significant when translating.  ARRAY2 is extended to length of
ARRAY1 by repeating its last character as necessary.  Excess characters
of ARRAY2 are ignored.  Character classes expand in unspecified order;
while translating, [:lower:] and [:upper:] may be used in pairs to
specify case conversion.  Squeezing occurs after translation or deletion.
`

var trOptions = []cli.Option{
	{Short: 'c', Long: "complement"},
	{Short: 'C', Key: "complement"},
	{Short: 'd', Long: "delete"},
	{Short: 's', Long: "squeeze-repeats"},
	{Short: 't', Long: "truncate-set1"},
}

// trKind is the kind of one element of a tr array.
type trKind int

const (
	trChars trKind = iota
	trClass
	trEquivalence
	trRepeat
)

// trElement is one element of a tr array: characters or a range, a class,
// an equivalence class, or a [c*n] repeat.
type trElement struct {
	kind  trKind
	chars []byte
	class string
	// count is the repeat count of [c*n]; fill marks [c*] and [c*0].
	count int
	fill  bool
}

// trClasses are the character classes of the C locale.
var trClasses = map[string]func(c byte) bool{
	"alnum":  func(c byte) bool { return isAlpha(c) || isDigit(c) },
	"alpha":  isAlpha,
	"blank":  func(c byte) bool { return c == ' ' || c == '\t' },
	"cntrl":  func(c byte) bool { return c < 32 || c == 127 },
	"digit":  isDigit,
	"graph":  func(c byte) bool { return c > 32 && c < 127 },
	"lower":  func(c byte) bool { return c >= 'a' && c <= 'z' },
	"print":  func(c byte) bool { return c >= 32 && c < 127 },
	"punct":  func(c byte) bool { return c > 32 && c < 127 && !isAlpha(c) && !isDigit(c) },
	"space":  func(c byte) bool { return c == ' ' || (c >= '\t' && c <= '\r') },
	"upper":  func(c byte) bool { return c >= 'A' && c <= 'Z' },
	"xdigit": func(c byte) bool { return isDigit(c) || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F') },
}

func isAlpha(c byte) bool {
	return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
}

func isDigit(c byte) bool {
	return c >= '0' && c <= '9'
}

func tr(inv *toolctx.Invocation, args []string) int {
	prog := &cli.Program{Name: args[0], Inv: inv, Usage: trUsage, BadUsage: 1, OptionsFirst: true}
	options, operands, status, done := prog.Parse(args[1:], trOptions)
	if done {
		return status
	}
	var complement, remove, squeeze, truncate bool
	for _, option := range options {
		switch option.Key {
		case "complement":
			complement = true
		case "delete":
			remove = true
		case "squeeze-repeats":
			squeeze = true
		case "truncate-set1":
			truncate = true
		}
	}
	translate := !remove && len(operands) == 2
	if err := checkTrOperands(operands, remove, squeeze); err != nil {
		return prog.UsageError(err)
	}
	set1, err := parseTrArray(prog, operands[0])
	if err != nil {
		prog.Errorf("%s", err)
		return 1
	}
	var set2 []trElement
	if len(operands) == 2 {
		set2, err = parseTrArray(prog, operands[1])
		if err != nil {
			prog.Errorf("%s", err)
			return 1
		}
	}
	program, err := buildTr(set1, set2, complement, remove, squeeze, truncate, translate)
	if err != nil {
		prog.Errorf("%s", err)
		return 1
	}
	return program.run(prog, inv)
}

// checkTrOperands checks the number of arrays against the options.
func checkTrOperands(operands []string, remove, squeeze bool) error {
	usage := func(format string, args ...any) error {
		return &cli.UsageError{Message: fmt.Sprintf(format, args...)}
	}
	if len(operands) == 0 {
		return usage("missing operand")
	}
	needTwo := !remove && !squeeze || remove && squeeze
	if len(operands) == 1 && needTwo {
		why := "Two strings must be given when translating."
		if remove {
			why = "Two strings must be given when both deleting and squeezing repeats."
		}
		return usage("missing operand after %s\n%s", cli.Quote(operands[0]), why)
	}
	if len(operands) == 2 && remove && !squeeze {
		return usage("extra operand %s\nOnly one string may be given when deleting without squeezing repeats.", cli.Quote(operands[1]))
	}
	if len(operands) > 2 {
		return usage("extra operand %s", cli.Quote(operands[2]))
	}
	return nil
}

// parseTrArray parses one array into its elements. Warnings go to the
// program's standard error.
func parseTrArray(prog *cli.Program, text string) ([]trElement, error) {
	var elements []trElement
	for index := 0; index < len(text); {
		if text[index] == '[' {
			element, next, ok, err := parseTrBracket(prog, text, index)
			if err != nil {
				return nil, err
			}
			if ok {
				elements = append(elements, element)
				index = next
				continue
			}
		}
		c, next := trChar(prog, text, index)
		if next < len(text) && text[next] == '-' && next+1 < len(text) {
			high, after := trChar(prog, text, next+1)
			if high < c {
				return nil, fmt.Errorf("range-endpoints of %s are in reverse collating sequence order", cli.Quote(text[index:after]))
			}
			var chars []byte
			for value := int(c); value <= int(high); value++ {
				chars = append(chars, byte(value))
			}
			elements = append(elements, trElement{kind: trChars, chars: chars})
			index = after
			continue
		}
		elements = append(elements, trElement{kind: trChars, chars: []byte{c}})
		index = next
	}
	return elements, nil
}

// parseTrBracket parses [:class:], [=c=], [c*n] or [c*] at text[index].
// ok is false when the bracket is an ordinary character.
func parseTrBracket(prog *cli.Program, text string, index int) (element trElement, next int, ok bool, err error) {
	rest := text[index:]
	for _, delimiter := range []byte{':', '='} {
		if len(rest) < 2 || rest[1] != delimiter {
			continue
		}
		end := strings.Index(rest[2:], string(delimiter)+"]")
		if end < 0 {
			continue
		}
		name := rest[2 : 2+end]
		next = index + 2 + end + 2
		if delimiter == ':' {
			if _, known := trClasses[name]; !known {
				return element, 0, false, fmt.Errorf("invalid character class %s", cli.Quote(name))
			}
			return trElement{kind: trClass, class: name}, next, true, nil
		}
		if len(name) != 1 {
			return element, 0, false, fmt.Errorf("%s: equivalence class operand must be a single character", name)
		}
		return trElement{kind: trEquivalence, chars: []byte{name[0]}}, next, true, nil
	}
	if len(rest) < 2 {
		return element, 0, false, nil
	}
	c, afterChar := trChar(prog, text, index+1)
	if afterChar >= len(text) || text[afterChar] != '*' {
		return element, 0, false, nil
	}
	end := strings.IndexByte(text[afterChar:], ']')
	if end < 0 {
		return element, 0, false, nil
	}
	countText := text[afterChar+1 : afterChar+end]
	next = afterChar + end + 1
	element = trElement{kind: trRepeat, chars: []byte{c}}
	if countText == "" {
		element.fill = true
		return element, next, true, nil
	}
	base := 10
	if countText[0] == '0' {
		base = 8
	}
	count, parseErr := strconv.ParseInt(countText, base, 32)
	if parseErr != nil || count < 0 {
		return element, 0, false, fmt.Errorf("invalid repeat count %s in [c*n] construct", cli.Quote(countText))
	}
	element.count = int(count)
	element.fill = count == 0
	return element, next, true, nil
}

// trChar reads one possibly escaped character at text[index] and returns
// it with the index after it.
func trChar(prog *cli.Program, text string, index int) (byte, int) {
	if text[index] != '\\' {
		return text[index], index + 1
	}
	if index+1 == len(text) {
		prog.Errorf("warning: an unescaped backslash at end of string is not portable")
		return '\\', index + 1
	}
	next := text[index+1]
	if next >= '0' && next <= '7' {
		end := index + 1
		value := 0
		for end < len(text) && end < index+4 && text[end] >= '0' && text[end] <= '7' {
			value = value*8 + int(text[end]-'0')
			end++
		}
		if value > 255 {
			digits := text[index+1 : end]
			prog.Errorf("warning: the ambiguous octal escape \\%s is being\n\tinterpreted as the 2-byte sequence \\0%s, %c", digits, digits[:2], digits[2])
			value /= 8
			end--
		}
		return byte(value), end
	}
	escapes := map[byte]byte{'a': '\a', 'b': '\b', 'f': '\f', 'n': '\n', 'r': '\r', 't': '\t', 'v': '\v'}
	if value, ok := escapes[next]; ok {
		return value, index + 2
	}
	return next, index + 2
}

// expand returns the characters of an element; a fill repeat expands to
// fillCount copies.
func (e trElement) expand(fillCount int) []byte {
	switch e.kind {
	case trClass:
		var chars []byte
		for c := 0; c < 256; c++ {
			if trClasses[e.class](byte(c)) {
				chars = append(chars, byte(c))
			}
		}
		return chars
	case trRepeat:
		count := e.count
		if e.fill {
			count = fillCount
		}
		return []byte(strings.Repeat(string(e.chars), max(count, 0)))
	case trChars, trEquivalence:
	}
	return e.chars
}

// trProgram is what tr does to each byte.
type trProgram struct {
	translation [256]byte
	remove      [256]bool
	squeeze     [256]bool
}

// buildTr checks the arrays against the operation and builds the byte
// tables.
func buildTr(set1, set2 []trElement, complement, remove, squeeze, truncate, translate bool) (*trProgram, error) {
	for _, element := range set1 {
		if translate && element.kind == trRepeat {
			return nil, fmt.Errorf("the [c*] repeat construct may not appear in string1")
		}
	}
	chars1 := expandAll(set1, 0)
	if complement {
		chars1 = complementChars(chars1)
	}
	program := &trProgram{}
	for c := range program.translation {
		program.translation[c] = byte(c)
	}
	var chars2 []byte
	if len(set2) > 0 || translate {
		var err error
		chars2, err = expandSet2(set1, set2, len(chars1), translate)
		if err != nil {
			return nil, err
		}
	}
	if translate {
		if truncate && len(chars1) > len(chars2) {
			chars1 = chars1[:len(chars2)]
		}
		if len(chars2) == 0 && len(chars1) > 0 {
			return nil, fmt.Errorf("when not truncating set1, string2 must be non-empty")
		}
		if len(chars2) < len(chars1) {
			if last := set2[len(set2)-1]; last.kind == trClass {
				return nil, fmt.Errorf("when translating with string1 longer than string2,\nthe latter string must not end with a character class")
			}
			extension := strings.Repeat(string(chars2[len(chars2)-1]), len(chars1)-len(chars2))
			chars2 = append(chars2, extension...)
		}
		for index, c := range chars1 {
			program.translation[c] = chars2[index]
		}
	}
	if remove {
		for _, c := range chars1 {
			program.remove[c] = true
		}
	}
	if squeeze {
		last := chars1
		if len(set2) > 0 {
			last = chars2
		}
		for _, c := range last {
			program.squeeze[c] = true
		}
	}
	return program, nil
}

func expandAll(elements []trElement, fillCount int) []byte {
	var chars []byte
	for _, element := range elements {
		chars = append(chars, element.expand(fillCount)...)
	}
	return chars
}

// complementChars returns every byte not in chars, in ascending order.
func complementChars(chars []byte) []byte {
	var present [256]bool
	for _, c := range chars {
		present[c] = true
	}
	var complement []byte
	for c := 0; c < 256; c++ {
		if !present[c] {
			complement = append(complement, byte(c))
		}
	}
	return complement
}

// expandSet2 expands the second array, filling [c*] to the length of the
// first, and checks the constructs translation allows.
func expandSet2(set1, set2 []trElement, length1 int, translate bool) ([]byte, error) {
	fills := 0
	fixed := 0
	for _, element := range set2 {
		switch {
		case element.kind == trRepeat && element.fill:
			fills++
		case translate && element.kind == trEquivalence:
			return nil, fmt.Errorf("[=c=] expressions may not appear in string2 when translating")
		case translate && element.kind == trClass && element.class != "upper" && element.class != "lower":
			return nil, fmt.Errorf("when translating, the only character classes that may appear in\nstring2 are 'upper' and 'lower'")
		default:
			fixed += len(element.expand(0))
		}
	}
	if fills > 1 {
		return nil, fmt.Errorf("only one [c*] repeat construct may appear in string2")
	}
	chars2 := expandAll(set2, length1-fixed)
	if translate {
		if err := checkCaseAlignment(set1, set2, length1-fixed); err != nil {
			return nil, err
		}
	}
	return chars2, nil
}

// checkCaseAlignment checks that every [:upper:] or [:lower:] of the second
// array starts where a case class starts in the first.
func checkCaseAlignment(set1, set2 []trElement, fillCount int) error {
	starts := map[int]string{}
	position := 0
	for _, element := range set1 {
		if element.kind == trClass {
			starts[position] = element.class
		}
		position += len(element.expand(0))
	}
	position = 0
	for _, element := range set2 {
		if element.kind == trClass {
			class, ok := starts[position]
			if !ok || (class != "upper" && class != "lower") {
				return fmt.Errorf("misaligned [:upper:] and/or [:lower:] construct")
			}
		}
		position += len(element.expand(fillCount))
	}
	return nil
}

// run filters standard input to standard output.
func (p *trProgram) run(prog *cli.Program, inv *toolctx.Invocation) int {
	input := cli.Reader(inv.Context, inv.Stdin)
	stdout := cli.Writer(inv.Context, inv.Stdout)
	buffer := make([]byte, 64*1024)
	output := make([]byte, 0, len(buffer))
	previous := -1
	for {
		read, err := input.Read(buffer)
		output = output[:0]
		for _, c := range buffer[:read] {
			if p.remove[c] {
				continue
			}
			c = p.translation[c]
			if p.squeeze[c] && previous == int(c) {
				continue
			}
			previous = int(c)
			output = append(output, c)
		}
		if len(output) > 0 {
			if _, err := stdout.Write(output); err != nil {
				return prog.WriteError(err)
			}
		}
		if err == io.EOF {
			return 0
		}
		if err != nil {
			prog.Errorf("read error: %s", cli.Strerror(err))
			return 1
		}
	}
}
