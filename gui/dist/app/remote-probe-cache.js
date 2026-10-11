// Cache observations only; never initiate SSH or retain credentials/configuration.
const KEY = 'colm.remote-probes.v1';
const fields = ['host', 'root', 'username', 'port', 'auth', 'identity_file', 'maps', 'threads', 'scheduler', 'partition', 'account', 'walltime', 'cpus', 'memory_gb', 'nodes', 'fetch_vars', 'env_script', 'directives'];
const probeFields = ['hostname', 'os', 'arch', 'cpus', 'memory_gb', 'root_free_gb', 'cargo', 'gfortran', 'mpi', 'schedulers', 'queues', 'engine', 'problems'];

function identity(server) {
  const text = JSON.stringify(fields.map(key => [key, server[key] ?? null]));
  let a = 2166136261, b = 5381;
  for (let i = 0; i < text.length; i++) {
    a = Math.imul(a ^ text.charCodeAt(i), 16777619);
    b = Math.imul(b, 33) ^ text.charCodeAt(i);
  }
  return `${text.length}:${a >>> 0}:${b >>> 0}`;
}

function read() {
  try {
    const entries = JSON.parse(globalThis.localStorage?.getItem(KEY) || '[]');
    return Array.isArray(entries) ? entries : [];
  } catch { return []; }
}

export function cachedProbe(server) {
  if (!server) return null;
  const entry = read().find(item => item?.identity === identity(server));
  if (!entry || !Number.isFinite(entry.at) || (!entry.probe && !entry.error)) return null;
  return { probe: entry.probe, error: entry.error, at: entry.at };
}

export function recordProbe(server, probe, error = null) {
  const entry = {
    identity: identity(server),
    probe: error || !probe ? null : Object.fromEntries(probeFields.filter(key => probe[key] !== undefined).map(key => [key, probe[key]])),
    // Raw SSH errors may echo environment commands or credentials.
    error: error ? 'probe_failed' : null,
    at: Date.now(),
  };
  try {
    const entries = read().filter(item => item?.identity !== entry.identity);
    globalThis.localStorage?.setItem(KEY, JSON.stringify([...entries, entry].slice(-64)));
  } catch { /* Storage may be disabled; live probe output still works. */ }
  globalThis.dispatchEvent?.(new CustomEvent('colm:probe'));
}
