import { useEffect, useState } from 'react';
import { Dialog } from './ui';
import { listen } from '../lib/ipc';
import { useApp } from '../lib/app-state';

/** Beta warning shown in About and under the version in the sidebar. */
export function betaNote(en: boolean) {
  return en
    ? 'Beta: features are changing quickly and known or unknown defects remain. Keep backups of your data and cases, and verify results independently before formal research use.'
    : '测试版（Beta）：功能仍在快速迭代，当前可能存在较多已知或未知缺陷。请保留原始数据与算例备份，正式科研使用前务必独立核验结果。';
}

/**
 * The app's own About window. The system About panel drops the credits, so the menu item
 * "about-colm" emits `colm-about` with the release version and this dialog shows the signature.
 */
export default function AboutDialog({ fallbackVersion }: { fallbackVersion: string }) {
  const { state } = useApp();
  const en = state.language === 'en';
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    void listen<string>('colm-about', (payload) => setVersion(payload || fallbackVersion)).then(
      (stop) => {
        if (alive) unlisten = stop;
        else stop();
      },
    );
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [fallbackVersion]);
  return (
    <Dialog
      title="CoLM Desktop"
      open={version !== null}
      onOpenChange={(open) => {
        if (!open) setVersion(null);
      }}
    >
      <div className="about-card">
        <img className="about-logo" src="assets/colm-icon.png" alt="" />
        <p className="muted">
          {en ? 'Version' : '版本'} {version ?? fallbackVersion}
        </p>
        <p className="beta-note">{betaNote(en)}</p>
        <div className="about-signature">
          <strong>{en ? 'Developed and maintained by' : '开发与维护团队'}</strong>
          <div className="about-team">CoLM LSM Development Team</div>
          <div>School of Atmospheric Sciences, SYSU</div>
          <div className="about-contact">{en ? 'Contact' : '联系人'}</div>
          <div className="about-maintainer">Zhongwang Wei (魏忠旺)</div>
          <div className="about-email">weizhw6@mail.sysu.edu.cn</div>
        </div>
        <p className="muted mini">
          {en
            ? 'Copyright © CoLM LSM Development Team, School of Atmospheric Sciences, SYSU'
            : '版权所有：CoLM陆面模式开发团队，中山大学大气科学学院'}
        </p>
      </div>
    </Dialog>
  );
}
