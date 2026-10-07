//! 不值得单独成模块的小东西。

export const $ = id => document.getElementById(id);

/** 状态栏。出错与「已保存」都走这里，免得两种消息各写各的。
 *
 *  直接写 DOM 而不是 import shell.js —— shell 要 import state，
 *  而几乎每个模块都 import ui，绕一圈就成了循环依赖。 */
export function status(msg) {
  $('status').textContent = String(msg);
}

/** 应用内的确认 / 输入对话框。
 *
 *  **不用 `window.confirm` / `prompt`**：桌面窗口里的 WebView 不弹系统对话框，直接当作“取消”返回，
 *  用户连按钮都看不到（实测助手的“数据外发确认”与 Study 导出因此一点就“已取消”）。
 *  文案由调用方给（已按语言处理）；按钮文字交给全局翻译。 */
function dialog(message, { input = null, okText = '确定', cancelText = '取消' } = {}) {
  return new Promise(resolve => {
    const box = document.createElement('dialog');
    box.className = 'app-dialog';
    const text = document.createElement('p');
    text.className = 'app-dialog-text';
    text.textContent = String(message);
    box.appendChild(text);
    let field = null;
    if (input !== null) {
      field = document.createElement('input');
      field.className = 'input';
      field.value = input;
      box.appendChild(field);
    }
    const row = document.createElement('div');
    row.className = 'app-dialog-actions';
    const cancel = document.createElement('button');
    cancel.type = 'button';
    cancel.className = 'btn-ghost';
    cancel.textContent = cancelText;
    const ok = document.createElement('button');
    ok.type = 'button';
    ok.className = 'run-btn';
    ok.textContent = okText;
    if (cancelText === null) row.append(ok);
    else row.append(cancel, ok);
    box.appendChild(row);
    let settled = false;
    const finish = value => {
      if (settled) return;
      settled = true;
      box.close();
      box.remove();
      resolve(value);
    };
    ok.onclick = () => finish(field ? field.value : true);
    cancel.onclick = () => finish(field ? null : false);
    box.addEventListener('cancel', event => { event.preventDefault(); finish(field ? null : false); });
    box.addEventListener('keydown', event => {
      if (event.key === 'Enter' && !event.isComposing && event.target === field) {
        event.preventDefault();
        finish(field.value);
      }
    });
    document.body.appendChild(box);
    box.showModal();
    (field ?? ok).focus();
  });
}

/** 确认：返回 true / false。 */
export function appConfirm(message, options) {
  return dialog(message, options);
}

/** 只有“确定”的提示。 */
export function appAlert(message) {
  return dialog(message, { cancelText: null }).then(() => undefined);
}

/** 输入：返回输入的文字，取消时返回 null。 */
export function appPrompt(message, value = '', options) {
  return dialog(message, { ...options, input: String(value ?? '') });
}


/** 路径的最后一段。
 *
 *  **两种分隔符都要认。** Windows 上算例目录是 `C:\Users\…\CN-Cng`，
 *  只按 `/` 切会原样返回整条路径 —— 于是横幅上写的不是「CN-Cng」而是
 *  一长串绝对路径，把那一行挤没。
 */
export function baseName(p) {
  return String(p).replace(/[\\/]+$/, '').split(/[\\/]/).pop();
}

/** 拼一段路径。分隔符跟着**已有那部分**走，两边都不认识平台。
 *
 *  Windows 的 API 认正斜杠，所以拼错了多半也能跑；但写进 namelist 的
 *  路径会被交给 `cmd`，而它在未加引号的参数里把 `/` 当开关前缀 ——
 *  这个坑刚在内核那边踩过一次。
 */
export function joinPath(dir, name) {
  const d = String(dir).replace(/[\\/]+$/, '');
  return d + (d.includes('\\') && !d.includes('/') ? '\\' : '/') + name;
}

/** 站点目录旁边按 CoLM 数据集约定放置的强迫场目录。
 *
 * 这和 `colm-cli scan` 的默认规则保持一致：站点文件所在目录的兄弟目录
 * `Forcing`。切换数据集时不能继续沿用上一套数据的强迫场路径，否则扫描
 * 会得到“站点存在、强迫场全不存在”的假象。
 */
export function forcingDirectoryForSiteDirectory(dir) {
  const d = String(dir).trim().replace(/[\\/]+$/, '');
  const split = Math.max(d.lastIndexOf('/'), d.lastIndexOf('\\'));
  if (split < 0) return '';
  return joinPath(d.slice(0, split), 'Forcing');
}

/** 自带示例各服务一种入口；普通用户站点不受这个筛选影响。 */
export function matchesBundledExampleMode(path, mode) {
  const name = baseName(path).toLowerCase();
  if (name === 'at-neu_2002-2012_fluxnet2015_site.nc') return false;
  const required = name === 'cn-cng_2008-2009_fluxnet2015_site.nc' ? 'natural'
    : name === 'at-neu_2010-2012_fluxnet-ch4_site.nc' ? 'methane'
      : name === 'us-ne3_2002-2003_fluxnet2015_crop_site.nc' ? 'crop'
        : name === 'au-preston_site_v1.nc' ? 'urban' : null;
  return required === null || required === mode;
}
