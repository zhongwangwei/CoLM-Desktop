import { useEffect } from 'react';
import { useApp } from '../lib/app-state';
import appManifest from '../../package.json';

/**
 * 启动封面：logo、名称、版本号，盖住后端与内核就绪之前的空白（旧界面的 loadinggate，0.3.0-beta.4 重建时丢了）。
 * 与 index.html 里的静态封面同一套类名：JS 还没加载时是静态的，React 挂载后接着由这里显示，不闪。
 * 20 秒还没好就放手：后端卡住时用户该看到界面和报错，而不是永远被封面挡着。
 */
export default function StartCover() {
  const { state, update } = useApp();
  const en = state.language === 'en';
  useEffect(() => {
    if (!state.starting) return;
    const timer = window.setTimeout(() => update({ starting: false }), 20_000);
    return () => window.clearTimeout(timer);
  }, [state.starting, update]);
  if (!state.starting) return null;
  return (
    <div className="start-cover" role="status" aria-live="polite">
      <div className="start-cover-panel">
        <img className="start-cover-logo" src="assets/colm-logo.png" alt="CoLM" />
        <h1>CoLM Desktop</h1>
        <p className="start-cover-version">v{appManifest.version}</p>
        <p className="start-cover-status">{en ? 'Loading…' : '正在加载…'}</p>
      </div>
    </div>
  );
}
