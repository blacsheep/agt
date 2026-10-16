/* How the page reaches its API: fetch from `agt web`, or the desktop app's
 * console in-process through Tauri. Both return a fetch-like response whose
 * body can be read as a stream. */
window.agtFetch = (() => {
  const tauri = window.__TAURI__?.core;
  if (!tauri) return (url, options) => fetch(url, options);
  return async (url, options = {}) => {
    const queue = [];
    let wake = null, done = false, failure = null, id = null;
    const notify = () => { const resume = wake; wake = null; resume?.(); };
    const channel = new tauri.Channel();
    channel.onmessage = message => {
      if (message.chunk !== undefined) queue.push(message.chunk);
      else if (message.error) failure = new Error(message.error);
      else done = true;
      notify();
    };
    const stop = () => { if (id !== null) tauri.invoke('agt_cancel', {id}); };
    options.signal?.addEventListener('abort', () => { failure = new DOMException('Aborted', 'AbortError'); stop(); notify(); });
    const started = await tauri.invoke('agt_request', {method: options.method || 'GET', path: url, body: options.body || '', channel});
    id = started.id;
    if (options.signal?.aborted) stop();
    const encoder = new TextEncoder();
    const read = async () => {
      while (!queue.length && !done && !failure) await new Promise(resolve => { wake = resolve; });
      if (queue.length) return {done: false, value: encoder.encode(queue.shift())};
      if (failure) throw failure;
      return {done: true, value: undefined};
    };
    const text = async () => {
      const decoder = new TextDecoder();
      let all = '';
      for (;;) { const part = await read(); if (part.done) return all + decoder.decode(); all += decoder.decode(part.value, {stream: true}); }
    };
    return {ok: started.status >= 200 && started.status < 300, status: started.status, json: async () => JSON.parse(await text()), text, body: {getReader: () => ({read, cancel: async () => stop()})}};
  };
})();
