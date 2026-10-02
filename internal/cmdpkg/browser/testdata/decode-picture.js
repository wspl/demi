async (encoded, codec) => {
  const data = Uint8Array.from(atob(encoded), (character) => character.charCodeAt(0));
  let decoder;
  const picture = await new Promise((resolve, reject) => {
    decoder = new VideoDecoder({ output: resolve, error: reject });
    decoder.configure({ codec, optimizeForLatency: true });
    decoder.decode(new EncodedVideoChunk({ type: 'key', timestamp: 0, data }));
    // Every picture is out before the flush resolves.
    decoder.flush().then(() => reject(new Error('the frame decoded to no picture')), reject);
  });
  decoder.close();
  const canvas = new OffscreenCanvas(picture.displayWidth, picture.displayHeight);
  canvas.getContext('2d', { alpha: false }).drawImage(picture, 0, 0);
  picture.close();
  const png = await canvas.convertToBlob({ type: 'image/png' });
  const url = await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(png);
  });
  globalThis.decodedPicture = url.slice(url.indexOf(',') + 1);
  return globalThis.decodedPicture.length;
}