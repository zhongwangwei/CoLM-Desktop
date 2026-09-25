#!/usr/bin/env bash
# 把 `oracle/scripts/compare_*.sh` 里的**模块级差分闭环**一次全跑。
#
# 为什么要有它：第 369 轮之前，每一轮只跑与当轮改动相关的那一个闭环，于是
# `compare_sortin` / `compare_stomata` / `compare_update_photosyn` 三个闭环
# **一直在失败**却没人知道 —— `compare_sortin.sh` 的头注释还把失败原因记反了
# （"`sortin` 是这条链上唯一没有 FMA 的函数"）。闭环是"形状对不对"的唯一判据
# （黄金窗口是混沌的），所以它必须能一键全跑、一眼看出谁在失败。
#
# 需要位置参数的**整例对照**分两类：
#   * `compare_second_config.sh <case>`（Campbell 土水 + 关 VSF，第 380 轮起**纳入本脚本**，
#     判据是"超容差变量数为 0"，跑不了的算例按 forcing 文件在不在逐个说明）；
#   * `compare_flag_isolated.sh <tag> "<namelist 行>"` 是**诊断探针**（用开关分区，
#     没有通过/失败判据），仍然跳过并列出。
#
# 用法:
#   oracle/scripts/compare_all.sh              # 全跑
#   oracle/scripts/compare_all.sh stomata      # 只跑名字里含 "stomata" 的
#
# 退出码：有任一闭环或整例对照失败 → 1（可以直接挂进 CI）。
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
    compare_flag_isolated|compare_switch_paths)
      # 两个都是**诊断探针**（不判通过/失败）：`compare_flag_isolated` 用开关分区，
      # `compare_switch_paths` 找非默认开关支路的首差 —— 后者现在还能找出两个
      # **未修**的种子（见 docs 第 381 轮），所以不能当门禁。
      skipped+=("$name")
      continue
      ;;
    compare_second_config)
      # 第 380 轮起不在这里跳过：它由下面的"整例对照"一节带算例名跑。
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

# ---------- 整例对照：第二配置（Campbell 土水 + 关 VSF） ----------
# 判据不是逐位相同（整例是混沌的），而是"超容差变量数为 0"。
# 跑不了的算例**逐个说出为什么**（本机 `oracle/work/US-NR1-snow` 要的
# `US-NR1_1999-2014_FLUXNET2015_Met.nc` 不在 `examples/Forcing/` 里）。
whole_pass=0
whole_fail=0
whole_skip=()
if [ -z "$filter" ]; then
  for case in CN-Cng CN-Cng-wet US-NR1-snow; do
    fprefix=$(grep -oE "fprefix\(1\)[[:space:]]*=[[:space:]]*'[^']+'" \
                "oracle/work/$case/forcing.nml" 2>/dev/null | head -1 | sed -E "s/.*'([^']+)'.*/\1/")
    if [ -z "$fprefix" ] || [ ! -f "examples/Forcing/$fprefix" ]; then
      whole_skip+=("$case(缺 examples/Forcing/${fprefix:-?})")
      continue
    fi
    out=$(bash oracle/scripts/compare_second_config.sh "$case" 2>&1)
    rc=$?
    last=$(printf '%s\n' "$out" | grep -E 'outside tolerance|^!!' | tail -1 | cut -c1-140)
    if [ "$rc" -eq 0 ]; then
      whole_pass=$((whole_pass + 1))
      printf '  ok   %-30s %s\n' "second_config:$case" "$last"
    else
      whole_fail=$((whole_fail + 1))
      failed+=("second_config:$case")
      printf '  FAIL %-30s %s\n' "second_config:$case" "$last"
    fi
  done
fi
echo "整例对照（第二配置）: ${whole_pass} 通过 / ${whole_fail} 失败"
if [ "${#whole_skip[@]}" -gt 0 ]; then
  echo "整例对照跳过: ${whole_skip[*]}"
fi
if [ "${#skipped[@]}" -gt 0 ]; then
  echo "跳过（诊断探针，不判通过/失败）: ${skipped[*]}"
fi
if [ $((fail + whole_fail)) -ne 0 ]; then
  printf '失败: %s\n' "${failed[*]}"
  exit 1
fi
