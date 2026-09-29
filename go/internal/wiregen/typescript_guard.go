package wiregen

import "fmt"

// refuseTypeScript refuses a package whose wire types the TypeScript emitter
// has no schema for: an adjacently tagged union and a struct with an inline
// union. The backend's browser-facing packages have neither; the machine
// manager's socket is not browser-facing.
func (p *Package) refuseTypeScript() error {
	for _, u := range p.Unions {
		if u.ContentName != "" {
			return fmt.Errorf("%s: an adjacently tagged union has no TypeScript schema", u.Name)
		}
	}
	for _, s := range p.Structs {
		for _, f := range s.Fields {
			if f.Inline {
				return fmt.Errorf("%s.%s: an inline union has no TypeScript schema", s.Name, f.Name)
			}
		}
	}
	return nil
}
