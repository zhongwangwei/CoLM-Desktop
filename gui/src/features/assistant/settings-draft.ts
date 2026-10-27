import { type SaveState } from '../../lib/save-status';
import { API_PROVIDERS, parseApiOptions, providerId, providerLabel, type Settings } from './model';

/**
 * 设置草稿的纯逻辑（U-07）：对话框里编辑的是一份草稿，而真正发送用的是已保存设置。
 * 这三件事分开算：草稿是否偏离已保存值、保存徽标三态、以及"实际发送：服务/模型"摘要。
 * 抽成纯函数是为了能在无 DOM 的测试里验证状态机，也避免面板与设置对话框各写一套比较逻辑。
 */

/** 与键顺序无关的 JSON 等值比较：设置对象经过后端往返，字段顺序不保证一致。 */
export function equalJson(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (Array.isArray(left) || Array.isArray(right))
    return (
      Array.isArray(left) &&
      Array.isArray(right) &&
      left.length === right.length &&
      left.every((value, index) => equalJson(value, right[index]))
    );
  if (left && right && typeof left === 'object' && typeof right === 'object') {
    const a = left as Record<string, unknown>;
    const b = right as Record<string, unknown>;
    const keys = Object.keys(a);
    return keys.length === Object.keys(b).length && keys.every((key) => equalJson(a[key], b[key]));
  }
  return false;
}

/** 解析"额外 API 选项"文本框；非法 JSON 返回 null，草稿按未保存处理。 */
export function draftApiOptions(text: string): Record<string, unknown> | null {
  try {
    return parseApiOptions(text);
  } catch {
    return null;
  }
}

function withoutOptions(settings: Settings): Omit<Settings, 'api_options'> {
  const copy: Partial<Settings> = { ...settings };
  delete copy.api_options;
  return copy as Omit<Settings, 'api_options'>;
}

/** 草稿（含文本框里的 API 选项）是否和已保存设置不同。 */
export function settingsDraftDirty(saved: Settings, draft: Settings, optionsText: string): boolean {
  const options = draftApiOptions(optionsText);
  if (!options) return true;
  return (
    !equalJson(withoutOptions(draft), withoutOptions(saved)) ||
    !equalJson(options, saved.api_options)
  );
}

/** 保存徽标三态：保存请求在飞时优先显示"正在保存"，否则看草稿是否脏。 */
export function draftSaveState(dirty: boolean, saving: boolean): SaveState {
  return saving ? 'saving' : dirty ? 'dirty' : 'saved';
}

const EXTERNAL_BACKENDS: Record<string, string> = {
  codex: 'Codex',
  claude_code: 'Claude Code',
  opencode: 'OpenCode',
};

/**
 * 设置对话框顶部显示的"实际发送：服务/模型"。必须针对**已保存**设置计算：
 * 草稿里换了服务或模型，在点保存之前发送仍走旧值，界面不能骗用户。
 */
export function sendingSummary(
  saved: Settings,
  language: string,
): { service: string; model: string } {
  if (saved.backend === 'builtin') {
    const provider = API_PROVIDERS.find((item) => item.id === providerId(saved));
    return {
      service: provider ? providerLabel(provider, language) : saved.base_url || 'API',
      model: saved.model || (language === 'en' ? 'provider default' : '服务默认'),
    };
  }
  return {
    service: EXTERNAL_BACKENDS[saved.backend] ?? saved.backend,
    model:
      saved.external[saved.backend]?.model || (language === 'en' ? 'default model' : '默认模型'),
  };
}
