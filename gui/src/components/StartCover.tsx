import { useEffect, useState } from 'react';
import { useApp } from '../lib/app-state';
import { Button } from './ui';
import appManifest from '../../package.json';

/**
 * 启动封面：logo、名称、版本号，盖住后端与内核就绪之前的空白（旧界面的 loadinggate，0.3.0-beta.4 重建时丢了）。
 * 与 index.html 里的静态封面同一套类名：JS 还没加载时是静态的，React 挂载后接着由这里显示，不闪。
 * 启动失败时封面不消失：显示失败阶段与原因，并给"重试启动"——原来 20 秒后无声撤掉封面，
 * 用户只会看到一排永远灰着的按钮，既不知道失败在哪也没法重试（GUI 审查 2026-10-22 P1-3）。
 */
export default function StartCover() {
  const { state, update, retryStartup } = useApp();
  const en = state.language === 'en';
  const startup = state.startup;
  const failed = startup?.phase === 'failed';
  const booting = !failed && (state.starting || startup?.phase === 'booting');
  const [waited, setWaited] = useState(0);
  useEffect(() => {
    if (!booting) {
      setWaited(0);
      return;
    }
    const timer = window.setInterval(() => setWaited((value) => value + 1), 1000);
    return () => window.clearInterval(timer);
  }, [booting]);
  if (!failed && !booting) return null;
  const stage =
    startup?.stage === 'kernels'
      ? en
        ? 'Reading model kernels…'
        : '正在读取模型内核…'
      : startup?.stage === 'versions'
        ? en
          ? 'Reading model versions…'
          : '正在读取模型版本…'
        : en
          ? 'Connecting to the backend…'
          : '正在连接后台…';
  return (
    <div
      className="start-cover"
      role={failed ? 'alert' : 'status'}
      aria-live={failed ? 'assertive' : 'polite'}
    >
      <div className="start-cover-panel">
        <img className="start-cover-logo" src="assets/colm-logo.png" alt="CoLM" />
        <h1>CoLM Desktop</h1>
        <p className="start-cover-version">v{appManifest.version}</p>
        {failed ? (
          <>
            <p className="start-cover-status">
              {en ? 'The app could not finish starting.' : '应用没有启动完成。'}
            </p>
            <p className="start-cover-error">{startup?.error}</p>
            <div className="button-row">
              <Button variant="primary" onClick={retryStartup}>
                {en ? 'Retry startup' : '重试启动'}
              </Button>
              {/* 失败不该把整个应用挡住：设置、历史、计算资源（以及另一半读到的内容）仍然能用。 */}
              <Button onClick={() => update({ startup: undefined })}>
                {en ? 'Continue with limited features' : '先进入应用（部分功能不可用）'}
              </Button>
            </div>
          </>
        ) : (
          <>
            <p className="start-cover-status">
              {stage}
              {waited > 0 ? (en ? ` ${waited}s` : ` 已用 ${waited} 秒`) : ''}
            </p>
            {waited >= 30 && (
              <p className="start-cover-status">
                {en
                  ? 'This is taking longer than usual; it will report an error and offer a retry rather than stay here forever.'
                  : '比平时慢；仍无法完成时会显示错误并给出重试，不会一直停在这里。'}
              </p>
            )}
          </>
        )}
      </div>
    </div>
  );
}
