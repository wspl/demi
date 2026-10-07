window.isolationReady = true;
window.isolation = {
  state() {
    return {isolated:crossOriginIsolated,shared:typeof SharedArrayBuffer,origin:location.origin};
  },
  async worker(type) {
    const buffer = new SharedArrayBuffer(4);
    const code = `${type === 'module' ? 'export {};' : ''}onmessage=e=>{Atomics.add(new Int32Array(e.data),0,7);postMessage({isolated:crossOriginIsolated,origin:location.origin});};`;
    const url = URL.createObjectURL(new Blob([code], {type:'text/javascript'}));
    let worker;
    let timer;
    try {
      worker = new Worker(url, {type});
      const state = await new Promise((resolve,reject) => {
        timer = setTimeout(()=>reject(new Error('Shared memory Worker timed out')),4000);
        worker.onerror = event => reject(new Error(event.message || 'Worker load failed'));
        worker.onmessage = event => resolve(event.data);
        worker.postMessage(buffer);
      });
      return {state,value:Atomics.load(new Int32Array(buffer),0)};
    } finally {
      clearTimeout(timer);
      worker?.terminate();
      URL.revokeObjectURL(url);
    }
  },
  async frames() {
    const child = document.createElement('iframe');
    let timer;
    try {
      await new Promise((resolve,reject) => {
        timer = setTimeout(()=>reject(new Error('Isolated child timed out')),4000);
        child.onload = resolve;
        child.src = '/lab/isolation?child=1&coep=' + encodeURIComponent(window.isolationCoep);
        document.body.append(child);
      });
      const target = child.contentWindow;
      target.document.body.dataset.parentWrite = 'written';
      return {isolated:target.crossOriginIsolated,written:target.document.body.dataset.parentWrite};
    } finally {
      clearTimeout(timer);
      child.remove();
    }
  },
  async resources() {
    const values = [];
    for (const corp of [true,false]) {
      const image = new Image();
      let timer;
      try {
        values.push(await new Promise((resolve,reject) => {
          timer = setTimeout(()=>reject(new Error('Isolation image timed out')),4000);
          image.onload = ()=>resolve('loaded');
          image.onerror = ()=>resolve('blocked');
          image.src = window.isolationRemote + '/lab/isolation-pixel' + (corp ? '?corp=1' : '');
          document.body.append(image);
        }));
      } finally {
        clearTimeout(timer);
        image.remove();
      }
    }
    return values;
  },
  popup() {
    const messages = [];
    let popup;
    let timer;
    const receive = event => {
      if (event.source === popup && event.origin === window.isolationRemote) messages.push(event.data);
    };
    window.addEventListener('message',receive);
    popup = window.open(window.isolationRemote + '/lab/isolation-callback');
    window.isolationPopupResult = new Promise((resolve,reject) => {
      const finish = () => {
        clearTimeout(timer);
        window.removeEventListener('message',receive);
        popup?.close();
      };
      timer = setTimeout(() => {
        finish();
        if (messages.length) resolve(messages);
        else reject(new Error('Popup sent no message'));
      },1500);
    });
  },
};
document.querySelector('#popup')?.addEventListener('click',window.isolation.popup);
