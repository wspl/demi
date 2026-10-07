// How a command matches text it was given a pattern for, such as `net cut`
// a request or `wait url` an address: `/regex/flags` as a regular
// expression, anything else as a part of the text. A path such as
// `/settings/keyboard` is a part of the text: what follows its last slash is
// not a set of flags.
export function matcher(pattern: string): (text: string) => boolean {
  const regex = /^\/(.+)\/([dgimsuvy]*)$/.exec(pattern)
  if (regex) {
    const expression = new RegExp(regex[1], regex[2])
    return (text) => expression.test(text)
  }
  return (text) => text.includes(pattern)
}
