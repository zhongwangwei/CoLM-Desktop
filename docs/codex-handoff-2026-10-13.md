# 给 Codex：React 重组审查修复后的交接（2026-10-13）

第 678–690 轮的未提交改动已经审查并修复，全部合并进本地 `main`（`51a10dda` 及之后）。审查发现和实测证据见 `docs/implementation-verification.md` 第 691 轮。

## 1. 先把工作目录切到 `main`

主检出目录里还留着修复前的未提交改动。它们和快照提交 `aba780ff` 内容相同，修复都在快照之后。先确认两者一致，再丢弃：

```bash
cd ~/Desktop/Github/CoLM-Desktop
git diff --stat aba780ff          # 应该没有输出：工作目录与快照一致
```

**确认没有差异之后**再执行：

```bash
git checkout -f main
git clean -fd -e .omx -e kernels  # 删掉快照里已入库的未跟踪副本；保留 .omx 记录和 kernels
```

如果 `git diff --stat aba780ff` 有输出，说明有快照之后的新改动：不要丢弃，先停下来问用户。

## 2. 这次改了的规则，接着做时要遵守

**版本**（`crates/colm-model/src/equivalence.rs`）：
- 两个配置是否算同一个版本，看 CoLM 实际生效的结果，不看 namelist 的字面写法。`FORCED_OFF` 里是从 `MOD_Namelist.F90` 提取的 12 条“父开关关闭时子开关被强制关闭”的规则；`forced_off_rules_match_colm` 测试会回头核对 Fortran 源码，上游改了要同步更新。
- 迁移时，没有实际差异的算例直接绑到官方版。新建算例和研究成员走 `migrate_case_to`，绑到指定的版本，不派生新版本。
- `freeze` 碰到效果相同的已有版本时，返回那个版本并删掉草稿。不要再加“每次点击都冻结一个新版本”的逻辑。
- 前端用 `features/versions/version-tree.ts`：每个版本只列最新修订、按来源成树；方案后缀用替换（`schemeVersionName`），不要追加。
- 首页没有门槛，官方版默认可用。

**用户自己定的界面，不要改**：顶栏那一行（CoLM Desktop · AI 助手 · English）、新建模拟的 5 步流程（范围与网格 → 数据与时段 → 模型与方案 → 预热与输出 → 检查与运行）。

**文字**：
- 组件里用 `t('中文', 'English')`；组件外（校验、IPC 错误、规则）用 `bilingual('中文', 'English')`（`lib/bilingual.ts`）。
- 不要再写 `'中文 / English'` 这种混在一个字符串里的提示。
- 日期按应用语言格式化（见 `history.ts` 里的 `appLocale`），不要用不带参数的 `toLocaleString()`。

**代码格式**：前端统一用 Prettier（`gui/.prettierrc.json`），CI 跑 `npm run format:check`。生成的 `src/lib/command-contract.ts` 不参与格式化，改完 IPC 用 `npm run contract` 重新生成。不要提交压缩成一行的代码。

**测试**：
- React 测试统一在英文环境下跑（`scripts/react-test-language.mjs`），断言写英文。
- 读页面源码的测试要用能识别多行 import 的写法 `/^import\b[\s\S]*?;$/gm`。页面里用到的新依赖要加进测试的注入列表。

**历史记录**：例行记录（`case-config-save`、`model-save-draft`、`model-migrate-case`、远程状态查询）默认不显示。新加类型时，在 `historyKindLabel` 里补中英文名。

**临时文件**：同一进程里可能同时有好几次操作，临时文件名除了进程号还要加计数器（参考 `pack_source`、`ssh.rs`）。

## 3. 接下来要做的

1. **原生验收**：重新打 Mac 包，在真实窗口里逐页过一遍。这次的修复只有组件测试和单元测试，没在原生窗口里看过。重点：
   - 首页直接可用；
   - “已有版本”弹窗显示成树；
   - 来回点方案卡片不会多出版本；
   - 历史记录列表和详情；
   - “关于 CoLM”菜单；
   - 结果图的时间轴。
2. **换成真正的 shadcn/ui 组件**：`components/ui/index.tsx` 现在是手写的薄封装。
3. **物理依赖规则移到 Rust 端**：`features/versions/scientific-setup.ts` 里的前提判断应由后端统一提供，前端只负责显示。
4. 用户本机版本库已清理，只剩官方版；被挪走的内容在 `~/Library/Application Support/CoLM-Desktop/models-cleanup-2026-10-13/`。之后的测试请用临时 registry，不要往用户真实的 registry 里写测试版本。

Codex 测试用 sol 模型。合并或推送 `main` 之前先问用户。
