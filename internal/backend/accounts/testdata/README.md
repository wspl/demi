# Accounts fixtures

These fixtures pin the bytes stored account data already holds. They are
recorded once; nothing in the repository regenerates them. All passwords here
are public fixture text, not credentials.

- `passwords.tsv`: password, base64 salt and Argon2id PHC string, as stored
  accounts hold them. `TestPasswordFixtures` verifies each hash and checks that
  `hashPassword` writes the same PHC bytes for the same salt.
- `go-password.tsv`: a password, raw salt and PHC string written by
  `hashPassword` and verified with a second, independent Argon2
  implementation. `TestPasswordWriterMatchesVerifiedFixture` checks that
  `hashPassword` still writes those bytes.
- `locales.tsv`: an input language tag and its ICU canonical form, or `ERROR`
  for a tag ICU refuses. `TestLocaleFixtures` checks the locale normalization
  against it.

`../iana_names.txt`, embedded by `locale.go`, lists every accepted normalized
time zone spelling from ICU's IANA parser, excluding unknown zones. An alias
keeps its own normalized spelling, not the canonical zone it refers to. It was
recorded once from ICU; nothing in the repository regenerates it.

These TSV records are fixture transport, not application JSON contracts.
