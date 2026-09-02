// Test stub for the net primitives, run with Bun: an HTTP + WebSocket server,
// the same over TLS when a certificate is given, and a CONNECT proxy.
// Prints "ready" once every listener is up.
const [httpPort, tlsPort, proxyPort] = (process.env.TINYJS_CONFORMANCE_PORTS ?? "").split(",").map(Number);
const cert = process.env.TINYJS_CONFORMANCE_CERT;
const key = process.env.TINYJS_CONFORMANCE_KEY;

const big = new Uint8Array(16 * 1024 * 1024).fill(120);

const fetchHandler = async (req: Request, server: any): Promise<Response> => {
  const url = new URL(req.url);
  if (url.pathname.startsWith("/ws")) {
    if (server.upgrade(req, { data: { path: url.pathname, received: 0 } })) return undefined as any;
    return new Response("upgrade failed", { status: 400 });
  }
  switch (url.pathname) {
    case "/hello":
      return new Response("hello", { headers: { "x-stub": "yes", "set-cookie": "a=1", "content-type": "text/plain" } });
    case "/big":
      return new Response(big);
    case "/chunked": {
      const stream = new ReadableStream({
        async start(c) {
          for (let i = 0; i < 5; i++) { c.enqueue(new TextEncoder().encode(`chunk${i};`)); await Bun.sleep(5); }
          c.close();
        },
      });
      return new Response(stream);
    }
    case "/headers":
      return new Response(JSON.stringify(Object.fromEntries(req.headers.entries())), { headers: { "content-type": "application/json" } });
    case "/upload": {
      let n = 0;
      let sum = 0;
      for await (const chunk of req.body as any) { n += chunk.length; for (const b of chunk) sum = (sum + b) & 0xffff; }
      return new Response(`got ${n} sum ${sum}`);
    }
    case "/status":
      return new Response("nope", { status: Number(url.searchParams.get("code") ?? "404") });
    default:
      return new Response("not found", { status: 404 });
  }
};

const websocket = {
  message(ws: any, message: any) {
    const data = typeof message === "string" ? new TextEncoder().encode(message) : new Uint8Array(message);
    ws.data.received += data.length;
    if (ws.data.path === "/ws") ws.send(data);
  },
  open(ws: any) {
    if (ws.data.path === "/ws-flood") {
      const frame = new Uint8Array(1024 * 1024).fill(7);
      let i = 0;
      const pump = () => {
        while (i < 64) {
          const r = ws.send(frame);
          i++;
          if (r === -1) return setTimeout(pump, 1);
        }
        ws.close(1000, "done");
      };
      pump();
    }
    if (ws.data.path === "/ws-close-now") ws.close(4001, "bye");
  },
  close(ws: any) {},
  drain(ws: any) {},
  maxPayloadLength: 64 * 1024 * 1024,
  backpressureLimit: 256 * 1024 * 1024,
  closeOnBackpressureLimit: false,
};

Bun.serve({ port: httpPort, hostname: "127.0.0.1", fetch: fetchHandler, websocket });
if (cert && key) {
  Bun.serve({ port: tlsPort, hostname: "127.0.0.1", tls: { cert: Bun.file(cert), key: Bun.file(key) }, fetch: fetchHandler, websocket });
}

// CONNECT proxy: replies 200 and pipes bytes both ways. Records tunnels in
// the process title so the suite can check the path was used.
let tunnels = 0;
Bun.listen({
  hostname: "127.0.0.1",
  port: proxyPort,
  socket: {
    data(socket: any, data: Uint8Array) {
      if (socket.data?.upstream) { socket.data.upstream.write(data); return; }
      const text = new TextDecoder().decode(data);
      const m = /^CONNECT ([^ :]+):(\d+) HTTP\/1\.1\r\n/.exec(text);
      if (!m) { socket.write("HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n"); socket.end(); return; }
      if (!/Proxy-Authorization: Basic dXNlcjpwYXNz\r\n/i.test(text)) { socket.write("HTTP/1.1 407 Proxy Authentication Required\r\ncontent-length: 0\r\n\r\n"); socket.end(); return; }
      tunnels++;
      socket.data = { pending: [] };
      Bun.connect({
        hostname: m[1], port: Number(m[2]),
        socket: {
          open(up: any) { socket.data.upstream = up; socket.write("HTTP/1.1 200 Connection established\r\n\r\n"); },
          data(_up: any, chunk: Uint8Array) { socket.write(chunk); },
          close() { socket.end(); },
          error() { socket.end(); },
        },
      }).catch(() => { socket.write("HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n"); socket.end(); });
    },
    close(socket: any) { socket.data?.upstream?.end(); },
    error() {},
  },
});
// The tunnel count is served on the plain HTTP port for the suite.
const origFetch = fetchHandler;
console.log("ready");
export { tunnels, origFetch };
