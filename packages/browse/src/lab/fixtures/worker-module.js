import { value } from './value.js';
self.onmessage = async () => {
  try { postMessage({ value, url: (await fetch('http://localhost:19401/lab/echo')).url }); }
  catch (error) { postMessage({ error: error.message }); }
};
