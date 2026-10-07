self.onmessage = async event => {
  try {
    if (event.data === 'fetch') postMessage({ url: (await fetch('http://localhost:19401/lab/echo')).url });
    else if (event.data === 'nested') {
      const child = new Worker('/lab/worker.js');
      child.onmessage = reply => { postMessage(reply.data); child.terminate(); };
      child.onerror = error => { postMessage({ error: error.message }); child.terminate(); };
      child.postMessage('fetch');
    } else if (event.data === 'ws') {
      const socket = new WebSocket('ws://localhost:19401/echo-ws', 'fixture-echo');
      socket.onopen = () => socket.send('worker-socket');
      socket.onmessage = message => { postMessage({ text: message.data, protocol: socket.protocol }); socket.close(); };
      socket.onerror = () => postMessage({ error: 'worker WebSocket failed' });
    }
  } catch (error) { postMessage({ error: error.message }); }
};
