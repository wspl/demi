for (const type of ['classic', 'module']) {
  add(`generated-shared-${type}-binary-source-and-revoke`, ['WORKER-01', 'ESCAPE-01'], async () => {
    const code = `${type === 'module' ? `import {value} from '${APP}/lab/worker-dependency.js';` : 'const value="worker-dependency";'}onconnect=e=>{const port=e.ports[0];port.onmessage=e=>port.postMessage({value,size:e.data.byteLength,trusted:e.isTrusted});port.start();};`;
    const bytes = new TextEncoder().encode(code);
    const blob = new Blob([bytes.subarray(0, 30), new Blob([bytes.subarray(30)])], {type: 'text/javascript'});
    const original = { text: await blob.text(), size: blob.size, type: blob.type };
    const url = URL.createObjectURL(blob);
    let worker;
    try {
      worker = new SharedWorker(url, {name:'binary-source',type});
      URL.revokeObjectURL(url);
      const data = new Uint8Array([1,2,3]);
      const result = await deadline(new Promise((resolve, reject) => {
        worker.onerror = e => reject(new Error(e.message || 'Shared Worker failed'));
        worker.port.onmessage = e => resolve({...e.data, trustedReply:e.isTrusted});
        worker.port.start();
        worker.port.postMessage(data.buffer, [data.buffer]);
      }));
      return {...result,detached:data.byteLength===0,blobUnchanged:original.text===await blob.text()&&original.size===blob.size&&original.type===blob.type};
    } finally {worker?.port.close();URL.revokeObjectURL(url);}
  });
  add(`generated-shared-${type}-cross-realm-sharing`, ['WORKER-01', 'ORIGIN-01'], () => useFrame('/lab/arrival', frame => useFrame('/lab/arrival', async second => {
    const source = `${type === 'module' ? 'export {};' : ''}let count=0;onconnect=e=>{const p=e.ports[0];p.onmessage=()=>p.postMessage(++count);p.start();};`;
    const url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
    const workers = [];
    try {
      const values = [];
      for (const child of [frame,second]) {
        const worker = new child.contentWindow.SharedWorker(url, {name:'cross-realm',type});
        workers.push(worker);
        values.push(await deadline(new Promise((resolve,reject)=>{
          worker.onerror=e=>reject(new Error(e.message || 'Shared Worker failed'));
          worker.port.onmessage=e=>resolve(e.data);
          worker.port.start();
          worker.port.postMessage('go');
        })));
      }
      return values;
    } finally {for(const worker of workers)worker.port.close();URL.revokeObjectURL(url);}
  })));
  add(`generated-shared-${type}-reconnect-revoked-url`, ['WORKER-01'], async () => {
    const url=URL.createObjectURL(new Blob([`${type==='module'?'export {};':''}let count=0;onconnect=e=>{const p=e.ports[0];p.onmessage=()=>p.postMessage(++count);p.start();};`],{type:'text/javascript'}));
    const workers=[];
    try {
      const values=[];
      for(const name of ['same','same','different']) {
        const worker=new SharedWorker(url,{type,name});
        workers.push(worker);
        values.push(await deadline(new Promise(resolve=>{
          worker.onerror=e=>{e.preventDefault();resolve('load-error');};
          worker.port.onmessage=e=>resolve(e.data);
          worker.port.start();
          worker.port.postMessage('go');
        })));
        URL.revokeObjectURL(url);
      }
      return values;
    } finally {for(const worker of workers)worker.port.close();URL.revokeObjectURL(url);}
  });
}
add('generated-data-module-dependency-runtime-order', ['WORKER-01','ESCAPE-01'], () => {
  const source=`import {origin,ready} from '${APP}/lab/generated-dependency.js';onmessage=async()=>postMessage({origin,remote:await ready,own:location.origin});`;
  return useWorker(`data:text/javascript,${encodeURIComponent(source)}`,{type:'module'},'go');
});
add('generated-data-module-large-source', ['WORKER-01'], () => {
  const source=`/*${'x'.repeat(128*1024)}*/onmessage=()=>postMessage('large-data-ready');`;
  return useWorker(`data:text/javascript,${encodeURIComponent(source)}`,{type:'module'},'go');
});
add('generated-shared-invalid-options-cleanup', ['WORKER-01','CLEAN-01'], () => {
  const url=URL.createObjectURL(new Blob(['onconnect=()=>{};'],{type:'text/javascript'}));
  try {new SharedWorker(url,{type:'not-a-worker-type'});return 'accepted';}
  catch(error){return error.name;}
  finally {URL.revokeObjectURL(url);}
}, {directExpected:'TypeError'});
add('generated-blob-original-resource-views', ['WORKER-01','HTTP-02'], async () => {
  const text='onmessage=()=>postMessage("raw-bytes");';
  const blob=new Blob([new TextEncoder().encode(text)],{type:'text/javascript'});
  const url=URL.createObjectURL(blob);
  try {
    const worker=await useWorker(url,{},'go');
    const response=await fetch(url);
    return {worker,text:await response.text(),sameURL:response.url===url,blobText:await blob.text(),size:blob.size};
  } finally {URL.revokeObjectURL(url);}
});
add('xfo-iframe-now-available', ['POLICY-01'], () => useFrame('/lab/policy?kind=xfo', frame => {
  try {return Boolean(frame.contentDocument?.querySelector('#policy'));}
  catch(error){if(error.name==='SecurityError')return false;throw error;}
}), {policyChange:'xfo-removed',directExpected:false,proxyExpected:true});
add('generated-worker-options-conversion', ['WORKER-01'], async () => {
  const results = [];
  for (const kind of ['Worker', 'SharedWorker']) {
    for (const option of [42, true, null, undefined, {extendedLifetime:true}, {sameSiteCookies:'invalid'}, Symbol('invalid'), 'named']) {
      const shared = kind === 'SharedWorker';
      const source = shared ? 'onconnect=e=>e.ports[0].postMessage(name);' : 'onmessage=()=>postMessage(name);';
      const url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
      let worker;
      try {
        worker = new window[kind](url, option);
        const port = shared ? worker.port : worker;
        const value = await deadline(new Promise(resolve => {
          worker.onerror = event => {event.preventDefault();resolve('load-error');};
          port.onmessage = event => resolve(event.data);
          if (shared) port.start();
          else port.postMessage('go');
        }));
        results.push({kind, value});
      } catch(error) {results.push({kind, error:error.name});}
      finally {
        if (shared) worker?.port.close();
        else worker?.terminate();
        URL.revokeObjectURL(url);
      }
    }
  }
  return results;
});
add('generated-worker-options-getter-order', ['WORKER-01'], async () => {
  const results = [];
  for (const kind of ['Worker', 'SharedWorker']) {
    const log = [];
    const options = new Proxy({}, {get(target,key) {
      log.push(key);
      if (key === 'credentials' || key === 'name' || key === 'type') return {toString() {
        log.push(`${key}:convert`);
        return key === 'credentials' ? 'same-origin' : key === 'type' ? 'classic' : 'ordered';
      }};
    }});
    const shared = kind === 'SharedWorker';
    const source = shared ? 'onconnect=e=>e.ports[0].postMessage(name);' : 'onmessage=()=>postMessage(name);';
    const url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
    let worker;
    try {
      worker = new window[kind](url, options);
      const port = shared ? worker.port : worker;
      const value = await deadline(new Promise((resolve,reject) => {
        worker.onerror = event => reject(new Error(event.message || 'Worker failed'));
        port.onmessage = event => resolve(event.data);
        if (shared) port.start();
        else port.postMessage('go');
      }));
      results.push({kind,log,value});
    } finally {
      if (shared) worker?.port.close();
      else worker?.terminate();
      URL.revokeObjectURL(url);
    }
  }
  return results;
});
add('generated-shared-distinct-utf16-names', ['WORKER-01'], async () => {
  const url = URL.createObjectURL(new Blob(['let count=0;onconnect=e=>e.ports[0].postMessage({count:++count,name});'], {type:'text/javascript'}));
  const workers = [];
  try {
    const values = [];
    for (const name of ['\ud800', '\ufffd', '\ud800']) {
      const worker = new SharedWorker(url, {name});
      workers.push(worker);
      values.push(await deadline(new Promise((resolve,reject) => {
        worker.onerror = event => reject(new Error(event.message || 'Shared Worker failed'));
        worker.port.onmessage = event => resolve(event.data);
        worker.port.start();
      })));
    }
    return values;
  } finally {
    for (const worker of workers) worker.port.close();
    URL.revokeObjectURL(url);
  }
});
add('generated-worker-syntax-error-timing', ['WORKER-01'], async () => {
  const results = [];
  for (const kind of ['Worker','SharedWorker']) {
    for (const type of ['classic','module']) {
      for (const protocol of kind === 'Worker' ? ['blob','data'] : ['blob']) {
        const source = 'function {';
        const url = protocol === 'blob' ? URL.createObjectURL(new Blob([source], {type:'text/javascript'})) : `data:text/javascript,${encodeURIComponent(source)}`;
        let worker;
        let constructed = false;
        try {
          const result = await deadline(new Promise(resolve => {
            try {
              worker = new window[kind](url,{type});
              worker.onerror = event => {
                event.preventDefault();
                resolve({constructed,trusted:event.isTrusted,event:event.type});
              };
              constructed = true;
            } catch(error) {resolve({constructed,error:error.name});}
          }));
          results.push({kind,type,protocol,...result});
        } finally {
          if(kind==='SharedWorker')worker?.port.close();
          else worker?.terminate();
          if(protocol==='blob')URL.revokeObjectURL(url);
        }
      }
    }
  }
  return results;
});
add('generated-shared-default-and-string-options-prototypes', ['WORKER-01'], async () => {
  const source = 'onconnect=e=>e.ports[0].postMessage(name);';
  const url = URL.createObjectURL(new Blob([source], {type:'text/javascript'}));
  const workers = [];
  const descriptors = [[Object.prototype,'name'],[String.prototype,'type']].map(([owner,key])=>[owner,key,Object.getOwnPropertyDescriptor(owner,key)]);
  let reads = 0;
  try {
    Object.defineProperty(Object.prototype,'name',{configurable:true,get(){reads++;return 'inherited';}});
    Object.defineProperty(String.prototype,'type',{configurable:true,get(){reads++;return 'module';}});
    const values = [];
    for (const option of [undefined, 'string-name']) {
      const worker = new SharedWorker(url,option);
      workers.push(worker);
      values.push(await deadline(new Promise((resolve,reject)=>{
        worker.onerror=event=>reject(new Error(event.message||'Shared Worker failed'));
        worker.port.onmessage=event=>resolve(event.data);
        worker.port.start();
      })));
    }
    return {reads,values};
  } finally {
    for (const [owner,key,descriptor] of descriptors) {
      if(descriptor)Object.defineProperty(owner,key,descriptor);
      else delete owner[key];
    }
    for (const worker of workers) worker.port.close();
    URL.revokeObjectURL(url);
  }
});
