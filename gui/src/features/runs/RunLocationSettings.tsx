import { Button, Field, Input, Notice, Select } from '../../components/ui';
import type { Server } from '../resources/resource';
import LiveServerLoad from './LiveServerLoad';

export default function RunLocationSettings({
  host,
  servers,
  disabled,
  error,
  en,
  onChange,
  onConfigure,
  serverNative = false,
  locationLocked = false,
}: {
  serverNative?: boolean;
  locationLocked?: boolean;
  host: string;
  servers: Server[] | null;
  disabled: boolean;
  error: string;
  en: boolean;
  onChange: (host: string) => void;
  onConfigure: () => void;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const server = servers?.find((item) => item.host === host);
  return (
    <section className="simulation-choice-group">
      <Field label={t('运行位置', 'Run location')}>
        <Select
          value={host}
          disabled={disabled || locationLocked || servers === null}
          onChange={(e) => onChange(e.target.value)}
        >
          <option value="">{t('本机', 'Local')}</option>
          {host && !server && (
            <option value={host}>
              {host} · {t('未配置', 'Not configured')}
            </option>
          )}
          {servers?.map((item) => (
            <option key={item.host} value={item.host}>
              {item.host}
            </option>
          ))}
        </Select>
      </Field>
      {error && <Notice tone="warning">{error}</Notice>}
      {host ? (
        <>
          {/* 选服务器时就看它忙不忙：负载、各分区空闲节点、全体排队和自己的作业。 */}
          {server && <LiveServerLoad host={host} partition={server.partition} en={en} />}
          <p className="muted">
            {serverNative
              ? t(
                  '输入数据直接从服务器选择，算例与生成结果保存在下方指定的服务器项目目录。',
                  'Choose input data directly on the server. Cases and generated results are saved in the server project folder below.',
                )
              : t(
                  '算例配置保存在本机；运行时同步到所选服务器。下方文件选择器浏览本机文件，输入数据通过路径对应引用服务器资料。',
                  'Case configuration is saved locally and synchronized to this server at run time. File pickers browse local files; input path mappings reference server datasets.',
                )}
          </p>
          {server && !serverNative && (
            <>
              <Field label={t('服务器工作根目录', 'Server working root')}>
                <Input readOnly value={server.root} />
              </Field>
              <Field
                label={t('输入路径对应（本机 → 服务器）', 'Input path mappings (local → server)')}
              >
                {server.maps.length ? (
                  <ul>
                    {server.maps.map((map) => (
                      <li key={map.local}>
                        {map.local} → {map.remote}
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="muted">
                    {t(
                      '尚未配置路径对应，请在运行前配置服务器已有数据的位置。',
                      'No path mappings configured. Map existing server datasets before running.',
                    )}
                  </p>
                )}
              </Field>
            </>
          )}
          {!server && servers !== null && (
            <Notice tone="warning">
              {t(
                '所选服务器未配置，请重新选择或配置连接。',
                'Configure or select an available server.',
              )}
            </Notice>
          )}
        </>
      ) : (
        <p className="muted">
          {t(
            '输入数据、算例配置和运行结果使用本机路径。',
            'Input data, case configuration and results use local paths.',
          )}
        </p>
      )}
      <Button disabled={disabled} onClick={onConfigure}>
        {serverNative
          ? t('配置计算资源', 'Configure resources')
          : t('配置计算资源与路径对应', 'Configure resources and path mappings')}
      </Button>
    </section>
  );
}
