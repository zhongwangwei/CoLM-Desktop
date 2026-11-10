#!/usr/bin/env python3
"""Generate manual appendices A/B after `cargo run -p xtask -- parameter-audit`."""
import argparse
import json
import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


def display_kind(kind: str) -> str:
    """Map the existing GUI Debug spelling only at the documentation boundary."""
    character = re.fullmatch(r"Character \{ len: (\d+) \}", kind)
    if character:
        return f"character(len={character[1]})"
    return {"Logical": "logical", "Integer": "integer", "Real": "real"}.get(kind, kind)


def md_escape(text) -> str:
    if text is None:
        return ''
    return str(text).replace('|', '\\|').replace('\n', ' ').strip()


def write_config_fields(data: Path, out: Path):
    fields = json.loads((data / 'fields.json').read_text(encoding='utf-8'))
    lines = [
        f'# 附录 A：配置字段总表（{len(fields)} 项）',
        '',
        '本表由 `xtask parameter-audit` 的 schema 字段表生成；类型与默认值使用 namelist 展示格式，覆盖桌面端当前 schema。默认值来自当前目录，并非冻结模型或运行算例的有效值。'
        '`文件` 列表示字段写进哪个 namelist（`nl_colm` = case.nml，`nl_colm_forcing` = forcing.nml，'
        '`nl_colm_history` = history.nml，其余为材料/同位素等扩展 namelist）。'
        '`派生` 为“是”的字段由程序覆盖，界面上应视为只读。'
        '`依赖宏` 表示该字段只有在对应编译宏开启的内核里才有意义。',
        '',
        '| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |',
        '|---|---|---|---|---|---|---|---|---|---|',
    ]
    rows = 0
    sections = sorted({f['section'] or '未分组' for f in fields})
    for section in sections:
        lines += [f'## {md_escape(section)}', '']
        subset = [f for f in fields if (f['section'] or '未分组') == section]
        subset.sort(key=lambda f: (f['group'] or '', f['name']))
        lines += [
            '| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |',
            '|---|---|---|---|---|---|---|---|---|---|',
        ]
        for f in subset:
            rows += 1
            values = ', '.join(f.get('values') or [])
            requires = ', '.join(f.get('requires') or [])
            doc = f.get('doc') or ''
            lines.append(
                '| {} | `{}` | {} | `{}` | {} | `{}` | {} | {} | {} | {} |'.format(
                    rows, md_escape(f['name']), md_escape(f.get('section') or '未分组'),
                    md_escape(f.get('group')), md_escape(display_kind(f['kind'])),
                    md_escape(f.get('default')), md_escape(values), md_escape(requires),
                    '是' if f.get('derived') else '', md_escape(doc),
                )
            )
        lines.append('')
    (out / 'appendix-config-fields.md').write_text('\n'.join(lines), encoding='utf-8')
    print('config fields:', rows)


def write_tuning_parameters(data: Path, out: Path):
    params = json.loads((data / 'catalog.json').read_text(encoding='utf-8'))
    lines = [
        f'# 附录 B：参数库索引（{len(params)} 项）',
        '',
        '本表由 `xtask parameter-audit` 的参数目录生成，覆盖模型可调参数（参数值、物理/过程参数、'
        '参数与资产来源）。`id` 是研究页与助手工具使用的稳定标识；`scope` 说明参数作用范围；'
        '`visibility` 是静态分类（`editable-common` = 常用类），实际可编辑性还取决于引擎、运行选项及写出能力。'
        '默认值来自源码声明，实数 kind 后缀已规范化；数组等值仍需对应作用域写入器处理。`默认提供方` 与 `源码位置` 指明来源。`写入目标` 显示 schema 字段的实际文件或引用该文件的键；其余参数由对应作用域的写入器处理。',
        '',
    ]
    by_section: dict[str, list[dict]] = {}
    for p in params:
        by_section.setdefault(p.get('section') or '未分组', []).append(p)
    for section in sorted(by_section):
        subset = sorted(by_section[section], key=lambda p: p['id'])
        lines += [f'## {md_escape(section)}（{len(subset)} 项）', '']
        lines += [
            '| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 写入目标 | 默认提供方 | 源码位置 |',
            '|---|---|---|---|---|---|---|---|---|---|---|',
        ]
        for i, p in enumerate(subset, 1):
            lines.append(
                '| {} | `{}` | `{}` | {} | `{}` | {} | {} | {} | {} | {} | {} |'.format(
                    i, md_escape(p['id']), md_escape(p.get('raw_key')),
                    md_escape(p.get('label_zh') or p.get('label_en')),
                    md_escape(p.get('default')), md_escape(p.get('unit')),
                    md_escape(p.get('scope')), md_escape(p.get('visibility')),
                    md_escape(p.get('storage_target')), md_escape(p.get('default_provider')),
                    md_escape(p.get('source_location')),
                )
            )
        lines.append('')
    (out / 'appendix-tuning-parameters.md').write_text('\n'.join(lines), encoding='utf-8')
    print('tuning parameters:', len(params))



if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, default=REPO / 'artifacts/parameter-audit')
    parser.add_argument('--out', type=Path, default=REPO / 'docs/user-manual/appendices')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    write_config_fields(args.data, args.out)
    write_tuning_parameters(args.data, args.out)
