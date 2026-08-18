# uPlot 1.6.31（vendored）

来源：`https://registry.npmjs.org/uplot/-/uplot-1.6.31.tgz`，MIT，见 `LICENSE`。

**随包分发而不是走 CDN**：`tauri.conf.json` 的 `script-src: 'self'` 禁止外部脚本。

选它的实测依据：
- `uPlot.iife.min.js` **50,312 字节**，`uPlot.min.css` 1,857 字节
- **零依赖**（`package.json` 的 `dependencies` 是空的）
- **Canvas 2D**：`getContext('2d')` 一处，`webgl` 与 `wasm` 各 0 次 —— 不需要 GPU，
  也不需要 WASM 加载
- IIFE 形式，不需要模块加载器，与 `script-src 'self'` 直接兼容

数据规模远在它的能力之内：一个站点两年 35088 个点，而它的实测能力是
166,650 点交互 25 毫秒。
