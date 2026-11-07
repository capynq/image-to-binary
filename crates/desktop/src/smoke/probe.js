(() => {
  const LENGTH = __ECHO_LENGTH__;
  const invoke = (command, payload) => window.__TAURI_INTERNALS__.invoke(command, payload);
  const state = { token: crypto.randomUUID(), violations: 0, workers: [] };
  window.__imageToBinarySmoke = state;
  const NativeWorker = window.Worker;
  window.Worker = function (url, options) {
    state.workers.push(String(url));
    return new NativeWorker(url, options);
  };
  document.addEventListener('securitypolicyviolation', () => {
    state.violations += 1;
  });
  const fnv1a64 = (bytes) => {
    let hash = 0xcbf29ce484222325n;
    for (const byte of bytes) {
      hash = ((hash ^ BigInt(byte)) * 0x100000001b3n) & 0xffffffffffffffffn;
    }
    return hash.toString(16).padStart(16, '0');
  };
  const patternBytes = (length) => {
    const bytes = new Uint8Array(length);
    for (let index = 0; index < length; index += 1) {
      bytes[index] = (index * 37 + 11) % 256;
    }
    return bytes;
  };
  const snapshot = (phase, extra) => ({
    phase,
    token: state.token,
    href: window.location.href,
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    devicePixelRatio: window.devicePixelRatio,
    cspViolations: state.violations,
    rootChildren: document.querySelectorAll('#root .app-shell').length,
    ...extra,
  });
  const send = (probe) => invoke('smoke_page', { probe });
  const startup = async () => {
    try {
      const appInfo = await invoke('app_info');
      const appLimits = await invoke('app_limits');
      const request = patternBytes(LENGTH);
      const response = new Uint8Array(await invoke('echo_binary', request));
      const expected = patternBytes(LENGTH);
      const matchesPattern = (bytes) =>
        bytes.length === expected.length && bytes.every((byte, index) => byte === expected[index]);
      const workerProbe = await invoke('smoke_worker_probe');
      await send(
        snapshot('startup', {
          appInfo,
          appLimits,
          echo: {
            bytes: request.length,
            requestFnv1a64: fnv1a64(request),
            responseFnv1a64: fnv1a64(response),
            requestMatchesPattern: matchesPattern(request),
            responseMatchesPattern: matchesPattern(response),
          },
          worker: workerProbe,
        }),
      );
    } catch (error) {
      await send(snapshot('startup', { error: JSON.stringify(error) }));
    }
  };
  window.__imageToBinarySmokeAfterResize = () => {
    send(snapshot('resize', {})).catch(() => {});
  };
  document.addEventListener('DOMContentLoaded', startup, { once: true });
})();
