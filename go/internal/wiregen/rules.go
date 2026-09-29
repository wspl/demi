package wiregen

import (
	"fmt"
	"go/constant"
	"strconv"
	"strings"
)

// parseRules reads the rules of a check tag.
func (p *reader) parseRules(tag string) ([]Rule, error) {
	items, err := splitTop(tag)
	if err != nil {
		return nil, err
	}
	var rules []Rule
	for _, item := range items {
		rule, err := p.parseRule(strings.TrimSpace(item))
		if err != nil {
			return nil, err
		}
		rules = append(rules, rule)
	}
	return rules, nil
}

// splitTop splits s at the commas that are not inside parentheses.
func splitTop(s string) ([]string, error) {
	var items []string
	depth, start := 0, 0
	for i, c := range s {
		switch c {
		case '(':
			depth++
		case ')':
			depth--
			if depth < 0 {
				return nil, fmt.Errorf("check %q: a parenthesis is closed that was not opened", s)
			}
		case ',':
			if depth == 0 {
				items = append(items, s[start:i])
				start = i + 1
			}
		}
	}
	if depth != 0 {
		return nil, fmt.Errorf("check %q: a parenthesis is not closed", s)
	}
	return append(items, s[start:]), nil
}

func (p *reader) parseRule(item string) (Rule, error) {
	if open := strings.IndexByte(item, '('); open >= 0 {
		name, inner := item[:open], item[open+1:]
		if !strings.HasSuffix(inner, ")") {
			return Rule{}, fmt.Errorf("check %q: text after the parenthesis", item)
		}
		kind := RuleKind(name)
		if kind != RuleEach && kind != RuleKeys {
			return Rule{}, fmt.Errorf("check %q: only each and keys take rules", item)
		}
		rules, err := p.parseRules(strings.TrimSuffix(inner, ")"))
		if err != nil {
			return Rule{}, err
		}
		return Rule{Kind: kind, Inner: rules}, nil
	}
	name, value, hasValue := strings.Cut(item, "=")
	kind := RuleKind(name)
	switch kind {
	case RuleNoNUL, RuleUnique, RuleNullable:
		if hasValue {
			return Rule{}, fmt.Errorf("check %q: %s takes no value", item, name)
		}
		return Rule{Kind: kind}, nil
	case RuleChars, RuleBytes, RuleItems, RuleRange:
		if !hasValue {
			return Rule{}, fmt.Errorf("check %q: %s takes MIN..MAX", item, name)
		}
		min, max, ok := strings.Cut(value, "..")
		if !ok || min == "" && max == "" {
			return Rule{}, fmt.Errorf("check %q: %s takes MIN..MAX, either bound may be left out", item, name)
		}
		rule := Rule{Kind: kind}
		var err error
		if rule.Min, err = p.bound(min); err != nil {
			return Rule{}, fmt.Errorf("check %q: %v", item, err)
		}
		if rule.Max, err = p.bound(max); err != nil {
			return Rule{}, fmt.Errorf("check %q: %v", item, err)
		}
		return rule, nil
	case RuleEq, RulePattern, RuleFunc:
		if !hasValue || value == "" {
			return Rule{}, fmt.Errorf("check %q: %s takes a value", item, name)
		}
		return Rule{Kind: kind, Value: value}, nil
	case RuleOneOf:
		if !hasValue || value == "" {
			return Rule{}, fmt.Errorf("check %q: oneof takes a|b", item)
		}
		return Rule{Kind: kind, Values: strings.Split(value, "|")}, nil
	}
	return Rule{}, fmt.Errorf("check %q: unknown rule %s", item, name)
}

// bound reads one end of a range: a number or the name of a constant; nothing
// leaves the end open.
func (p *reader) bound(text string) (*Bound, error) {
	if text == "" {
		return nil, nil
	}
	value, err := p.number(text)
	if err != nil {
		return nil, err
	}
	return &Bound{Src: text, Value: value}, nil
}

// number evaluates a literal or the name of a constant to an integer.
func (p *reader) number(text string) (int64, error) {
	if n, err := strconv.ParseInt(text, 10, 64); err == nil {
		return n, nil
	}
	value, ok := p.pkg.consts[text]
	if !ok {
		return 0, fmt.Errorf("%s is neither a number nor a constant of the package", text)
	}
	n, exact := constant.Int64Val(constant.ToInt(value))
	if !exact {
		return 0, fmt.Errorf("%s is not an integer", text)
	}
	return n, nil
}

// checkRules refuses a rule that does not apply to the type it is written on.
func (p *reader) checkRules(t *Type, rules []Rule) error {
	if t.Kind == KindPointer {
		t = t.Elem
	}
	for _, rule := range rules {
		if err := p.checkRule(t, rule); err != nil {
			return err
		}
	}
	return nil
}

func (p *reader) checkRule(t *Type, rule Rule) error {
	on := func(kinds ...Kind) error {
		for _, kind := range kinds {
			if t.Kind == kind {
				return nil
			}
		}
		return fmt.Errorf("the rule %s does not apply to %s", rule.Kind, t.Src)
	}
	switch rule.Kind {
	case RuleChars, RuleBytes, RuleNoNUL, RuleOneOf:
		return on(KindString)
	case RulePattern:
		if err := on(KindString); err != nil {
			return err
		}
		if _, ok := p.pkg.patterns[rule.Value]; !ok {
			return fmt.Errorf("pattern=%s: the package has no variable %s = regexp.MustCompile(\"...\")", rule.Value, rule.Value)
		}
	case RuleItems:
		return on(KindSlice, KindMap)
	case RuleUnique:
		if err := on(KindSlice); err != nil {
			return err
		}
		if k := t.Elem.Kind; k != KindString && k != KindInt && k != KindUint && k != KindBool {
			return fmt.Errorf("unique applies to a slice of strings, numbers or booleans")
		}
	case RuleRange:
		return on(KindInt, KindUint)
	case RuleEq:
		if err := on(KindString, KindInt, KindUint); err != nil {
			return err
		}
		if t.Kind != KindString {
			if _, err := p.number(rule.Value); err != nil {
				return fmt.Errorf("eq=%s: %v", rule.Value, err)
			}
		}
	case RuleFunc:
	case RuleEach:
		if err := on(KindSlice, KindMap); err != nil {
			return err
		}
		return p.checkRules(t.Elem, rule.Inner)
	case RuleKeys:
		if err := on(KindMap); err != nil {
			return err
		}
		return p.checkRules(t.Key, rule.Inner)
	}
	return nil
}
