/**
 * The standard reason phrases of HTTP's status codes (RFC 9110 § 15, with
 * 429 and 431 from RFC 6585 and 451 from RFC 7725), for an answer that
 * carried none, as HTTP/2's and HTTP/3's do: they send no reason phrase, and
 * the browser's `statusText` is then empty.
 */
const PHRASES: Readonly<Record<number, string>> = {
  100: 'Continue',
  101: 'Switching Protocols',
  200: 'OK',
  201: 'Created',
  202: 'Accepted',
  203: 'Non-Authoritative Information',
  204: 'No Content',
  205: 'Reset Content',
  206: 'Partial Content',
  300: 'Multiple Choices',
  301: 'Moved Permanently',
  302: 'Found',
  303: 'See Other',
  304: 'Not Modified',
  307: 'Temporary Redirect',
  308: 'Permanent Redirect',
  400: 'Bad Request',
  401: 'Unauthorized',
  402: 'Payment Required',
  403: 'Forbidden',
  404: 'Not Found',
  405: 'Method Not Allowed',
  406: 'Not Acceptable',
  407: 'Proxy Authentication Required',
  408: 'Request Timeout',
  409: 'Conflict',
  410: 'Gone',
  411: 'Length Required',
  412: 'Precondition Failed',
  413: 'Content Too Large',
  414: 'URI Too Long',
  415: 'Unsupported Media Type',
  416: 'Range Not Satisfiable',
  417: 'Expectation Failed',
  421: 'Misdirected Request',
  422: 'Unprocessable Content',
  426: 'Upgrade Required',
  428: 'Precondition Required',
  429: 'Too Many Requests',
  431: 'Request Header Fields Too Large',
  451: 'Unavailable For Legal Reasons',
  500: 'Internal Server Error',
  501: 'Not Implemented',
  502: 'Bad Gateway',
  503: 'Service Unavailable',
  504: 'Gateway Timeout',
  505: 'HTTP Version Not Supported',
  511: 'Network Authentication Required',
}

/**
 * An answer's status line as people read it, such as `500 Internal Server
 * Error`: its code and its reason phrase, the code's standard phrase where
 * the answer carried none, or the code alone for one no standard names.
 */
export function httpStatusLine(status: number, statusText = ''): string {
  const phrase = statusText.trim() || PHRASES[status]
  return phrase ? `${status} ${phrase}` : String(status)
}
