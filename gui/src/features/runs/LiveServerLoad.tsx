import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../lib/ipc';
import ServerLoadPanel from './ServerLoadPanel';
import type { ServerLoad } from './server-load';

/** The server status panel that reads by itself: once per chosen server, again on Refresh. */
export default function LiveServerLoad({
  host,
  partition,
  en,
}: {
  host: string;
  partition?: string;
  en: boolean;
}) {
  const [reading, setReading] = useState<{
    host?: string;
    at?: number;
    load?: ServerLoad;
    error?: string;
  }>({});
  const [busy, setBusy] = useState(false);
  // 来回切换服务器时，慢的那次回答不能盖住当前服务器的。
  const current = useRef(host);
  current.current = host;
  async function refresh() {
    const target = host;
    setBusy(true);
    try {
      const load = await invoke<ServerLoad>('remote_load', { host: target });
      if (current.current === target) setReading({ host: target, at: Date.now(), load });
    } catch (error) {
      if (current.current === target)
        setReading({ host: target, at: Date.now(), error: String(error) });
    } finally {
      if (current.current === target) setBusy(false);
    }
  }
  useEffect(() => {
    if (host) void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [host]);
  const mine = reading.host === host;
  return (
    <ServerLoadPanel
      host={host}
      load={mine ? reading.load : undefined}
      error={mine ? reading.error : undefined}
      at={mine ? reading.at : undefined}
      busy={busy}
      partition={partition}
      en={en}
      onRefresh={() => void refresh()}
    />
  );
}
