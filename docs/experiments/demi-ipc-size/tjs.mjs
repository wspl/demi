// Same transport probe as the C clients; no Demi protocol or command code.
if (tjs.args.length !== 2) {
  console.error('Usage: ipc-client endpoint');
  tjs.exit(2);
}
try {
  const socket = await tjs.connect('pipe', tjs.args[1]);
  socket.closed.catch(() => {});
  const { readable, writable } = await socket.opened;
  tjs.stdin.pipeTo(writable).catch(error => {
    console.error(error);
    tjs.exit(1);
  });
  await readable.pipeTo(tjs.stdout);
  tjs.exit(0);
} catch (error) {
  console.error(error);
  tjs.exit(1);
}
