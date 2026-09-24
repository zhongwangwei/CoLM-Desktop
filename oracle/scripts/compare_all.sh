#!/usr/bin/env bash
# 把 `oracle/scripts/compare_*.sh` 里的**模块级差分闭环**一次全跑。
#
# 为什么要有它：第 369 轮之前，每一轮只跑与当轮改动相关的那一个闭环，于是
# `compare_sortin` / `compare_stomata` / `compare_update_photosyn` 三个闭环
# **一直在失败**却没人知道 —— `compare_sortin.sh` 的头注释还把失败原因记反了
# （"`sortin` 是这条链上唯一没有 FMA 的函数"）。闭环是"形状对不对"的唯一判据
# （黄金窗口是混沌的），所以它必须能一键全跑、一眼看出谁在失败。
#
# 需要位置参数的**整例对照**（`compare_flag_isolated.sh` 要 `<tag> "<namelist 行>"`
# 和 `compare_second_config.sh` 要 `<case>`）不是闭环，跳过并列出。
#
# 用法:
#   oracle/scripts/compare_all.sh              # 全跑
#   oracle/scripts/compare_all.sh stomata      # 只跑名字里含 "stomata" 的
#
# 退出码：有任一闭环失败 → 1（可以直接挂进 CI）。
set -uo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$BASE"
filter="${1:-}"
pass=0
fail=0
failed=()
skipped=()
for f in oracle/scripts/compare_*.sh; do
  name="$(basename "$f" .sh)"
  case "$name" in
    compare_all)
      continue
      ;;
    compare_flag_isolated|compare_second_config)
      skipped+=("$name")
      continue
      ;;
  esac
  if [ -n "$filter" ]; then
    case "$name" in
      *"$filter"*) ;;
      *) continue ;;
    esac
  fi
  out=$(bash "$f" 2>&1)
  rc=$?
  # 摘要有信息量的一行：优先 "mismatch"（失败）/"identical"（通过），
  # 否则退回最后一行非空输出（有些闭环只打印分支分布）。
  last=$(printf '%s\n' "$out" | grep -E 'mismatch|identical|first kernel' | tail -1 | cut -c1-140)
  [ -n "$last" ] || last=$(printf '%s\n' "$out" | grep -vE '^\s*$' | tail -1 | cut -c1-140)
  if [ "$rc" -eq 0 ]; then
    pass=$((pass + 1))
    printf '  ok   %-30s %s\n' "$name" "$last"
  else
    fail=$((fail + 1))
    failed+=("$name")
    printf '  FAIL %-30s %s\n' "$name" "$last"
  fi
done
echo
echo "闭环: ${pass} 通过 / ${fail} 失败"
if [ "${#skipped[@]}" -gt 0 ]; then
  echo "跳过（需要位置参数，不是闭环）: ${skipped[*]}"
fi
if [ "$fail" -ne 0 ]; then
  printf '失败: %s\n' "${failed[*]}"
  exit 1
fi
