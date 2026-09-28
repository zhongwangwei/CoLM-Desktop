#!/usr/bin/env python3
"""给 Fortran `bgc_driver` 插桩，逐个子过程记录一个 patch 的全部 BGC 状态。

用法：`gen_bgc_trace.py <上游 main/BGC/MOD_BGC_driver.F90> <输出 .F90>`

输出文件 = 生成的 `MOD_BGC_Trace` 模块 + 插桩后的 driver。把它覆盖到调试构建树里的
`main/BGC/MOD_BGC_driver.F90` 再编 `colm.x`；运行时设 `COLM_BGC_TRACE=<文件>`（可选
`COLM_BGC_TRACE_CALLS=<每个窗口记录几次 driver 调用>`，默认 1；`COLM_BGC_TRACE_FROM=<第几次调用起>`，
默认 1；`COLM_BGC_TRACE_EVERY=<窗口间隔的调用数>`，默认只有一个窗口）即写出追踪。Rust 引擎在相同位置、
用相同格式写（`colm_core::bgc_trace`），`bgc_trace_cmp.py` 给出第一个分叉的（调用序号，阶段，
字段，下标）；Rust 的逐过程回放（`bgc_replay`）拿某阶段之前的记录当输入、只跑这一个过程，
再与之后的记录逐位比。

为什么要这个：BGC driver 一步串起约 30 个过程、改写约 1000 个数组，端到端只看重启文件
只能知道"错了"，不知道错在哪一个过程。

记录格式（小端流式，自描述）：每条记录 = 32 字节阶段名（空格补齐）+ 两段字段表，先是物理输入
（`EXTRAS`，driver 实参与 BGC 读写的非 BGC 变量），再是 BGC 状态（`gen_bgc_state.MODULES` 的
全部数组）。每个字段是 `[i32 名长][名][i32 n][n × f64]`，名长为 0 表示该段结束。未分配或宏关闭的
数组写 n=0；整数与逻辑量转成 f64（逻辑量 1/0）。
"""

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gen_bgc_state  # noqa: E402

# BGC 读写、且不属于 MOD_BGC_Vars_* 的物理量。`CNVegStructUpdate` 每步改写 tsai_p、lai、tlai，
# 它们回流到下一步的物理，所以必须一起比。
EXTRAS = [
    # driver 实参
    ("idate", None, "raw:real(idate,r8)"),
    ("deltim", None, "raw:[deltim]"),
    ("dlat", None, "raw:[dlat]"),
    ("dlon", None, "raw:[dlon]"),
    # 全局土壤网格
    ("z_soi", "MOD_Vars_Global", "raw:z_soi"),
    ("dz_soi", "MOD_Vars_Global", "raw:dz_soi"),
    ("zi_soi", "MOD_Vars_Global", "raw:zi_soi"),
    # PFT 物理量
    ("pftclass", "MOD_Vars_PFTimeInvariants", "raw:real(pftclass(ps:pe),r8)"),
    ("pftfrac", "MOD_Vars_PFTimeInvariants", "pft"),
    ("rootfr_p", "MOD_Const_PFT", "raw:rootfr_p(1:nl_soil,pftclass(ps:pe))"),
    ("tsai_p", "MOD_Vars_PFTimeVariables", "pft"),
    ("tlai_p", "MOD_Vars_PFTimeVariables", "pft"),
    ("lai_p", "MOD_Vars_PFTimeVariables", "pft"),
    ("laisun_p", "MOD_Vars_PFTimeVariables", "pft"),
    ("laisha_p", "MOD_Vars_PFTimeVariables", "pft"),
    ("sigf_p", "MOD_Vars_PFTimeVariables", "pft"),
    ("tref_p", "MOD_Vars_PFTimeVariables", "pft"),
    ("assim_p", "MOD_Vars_1DPFTFluxes", "pft"),
    ("respc_p", "MOD_Vars_1DPFTFluxes", "pft"),
    # patch 物理量
    ("patchclass", "MOD_Vars_TimeInvariants", "raw:real(patchclass(i:i),r8)"),
    ("patchlatr", "MOD_Vars_TimeInvariants", "patch"),
    ("smpmax_hr", "MOD_Vars_TimeInvariants", "raw:[smpmax_hr]"),
    ("smpmin_hr", "MOD_Vars_TimeInvariants", "raw:[smpmin_hr]"),
    ("porsl", "MOD_Vars_TimeInvariants", "soil"),
    ("psi0", "MOD_Vars_TimeInvariants", "soil"),
    ("bsw", "MOD_Vars_TimeInvariants", "soil"),
    ("theta_r", "MOD_Vars_TimeInvariants", "soil"),
    ("alpha_vgm", "MOD_Vars_TimeInvariants", "soil"),
    ("n_vgm", "MOD_Vars_TimeInvariants", "soil"),
    ("L_vgm", "MOD_Vars_TimeInvariants", "soil"),
    ("sc_vgm", "MOD_Vars_TimeInvariants", "soil"),
    ("fc_vgm", "MOD_Vars_TimeInvariants", "soil"),
    ("BD_all", "MOD_Vars_TimeInvariants", "soil"),
    ("wfc", "MOD_Vars_TimeInvariants", "soil"),
    ("OM_density", "MOD_Vars_TimeInvariants", "soil"),
    ("lai", "MOD_Vars_TimeVariables", "patch"),
    ("tlai", "MOD_Vars_TimeVariables", "patch"),
    ("tref", "MOD_Vars_TimeVariables", "patch"),
    ("t_soisno", "MOD_Vars_TimeVariables", "soil"),
    ("wliq_soisno", "MOD_Vars_TimeVariables", "soil"),
    ("wice_soisno", "MOD_Vars_TimeVariables", "soil"),
    ("smp", "MOD_Vars_TimeVariables", "soil"),
    ("h2osoi", "MOD_Vars_TimeVariables", "soil"),
    ("rsur", "MOD_Vars_1DFluxes", "patch"),
    ("rnof", "MOD_Vars_1DFluxes", "patch"),
    ("forc_t", "MOD_Vars_1DForcing", "patch"),
    ("forc_q", "MOD_Vars_1DForcing", "patch"),
    ("forc_psrf", "MOD_Vars_1DForcing", "patch"),
    ("forc_prc", "MOD_Vars_1DForcing", "patch"),
    ("forc_prl", "MOD_Vars_1DForcing", "patch"),
    ("forc_us", "MOD_Vars_1DForcing", "patch"),
    ("forc_vs", "MOD_Vars_1DForcing", "patch"),
    # 只有 BGC 历史/作物用到的诊断量（CNSummary 写）
    ("lai_enftemp", "MOD_Vars_TimeVariables", "patch"),
    ("lai_enfboreal", "MOD_Vars_TimeVariables", "patch"),
    ("lai_dnfboreal", "MOD_Vars_TimeVariables", "patch"),
    ("lai_ebftrop", "MOD_Vars_TimeVariables", "patch"),
    ("lai_ebftemp", "MOD_Vars_TimeVariables", "patch"),
    ("lai_dbftrop", "MOD_Vars_TimeVariables", "patch"),
    ("lai_dbftemp", "MOD_Vars_TimeVariables", "patch"),
    ("lai_dbfboreal", "MOD_Vars_TimeVariables", "patch"),
    ("lai_ebstemp", "MOD_Vars_TimeVariables", "patch"),
    ("lai_dbstemp", "MOD_Vars_TimeVariables", "patch"),
    ("lai_dbsboreal", "MOD_Vars_TimeVariables", "patch"),
    ("lai_c3arcgrass", "MOD_Vars_TimeVariables", "patch"),
    ("lai_c3grass", "MOD_Vars_TimeVariables", "patch"),
    ("lai_c4grass", "MOD_Vars_TimeVariables", "patch"),
    ("irrig_method_corn", "MOD_Vars_TimeVariables", "patch"),
    ("irrig_method_swheat", "MOD_Vars_TimeVariables", "patch"),
    ("irrig_method_wwheat", "MOD_Vars_TimeVariables", "patch"),
    ("irrig_method_soybean", "MOD_Vars_TimeVariables", "patch"),
    ("irrig_method_cotton", "MOD_Vars_TimeVariables", "patch"),
    ("irrig_method_rice1", "MOD_Vars_TimeVariables", "patch"),
    ("irrig_method_rice2", "MOD_Vars_TimeVariables", "patch"),
    ("irrig_method_sugarcane", "MOD_Vars_TimeVariables", "patch"),
    ("irrig_method_p", "MOD_Vars_PFTimeVariables", "raw:real(irrig_method_p(ps:pe),r8)"),
]


def extra_slice(name, kind):
    if kind.startswith("raw:"):
        return kind[4:]
    return {
        "pft": f"{name}(ps:pe)",
        "patch": f"{name}(i:i)",
        "soil": f"{name}(1:nl_soil,i)",
    }[kind]


def field_slice(name, shape):
    if shape[-1] == "numpatch":
        return f"{name}({','.join([':'] * (len(shape) - 1) + ['i:i'])})"
    if shape[-1] == "numpft":
        return f"{name}({','.join([':'] * (len(shape) - 1) + ['ps:pe'])})"
    return name


def emit_value(out, name, expr, kind):
    conv = {"f64": expr, "i32": f"real({expr},r8)", "bool": f"merge(1._r8,0._r8,{expr})"}[kind]
    out.append(f"      write(trace_unit) {len(name)}_4, '{name}', int(size({conv}),4), {conv}")


def emit_empty(out, name):
    out.append(f"      write(trace_unit) {len(name)}_4, '{name}', 0_4")


def guards(path):
    """每个 allocatable 声明外层的预处理条件（例如 `#ifdef CROP`），原样套在追踪语句外。"""
    stack, out = [], {}
    for line in path.read_text(errors="replace").splitlines():
        s = line.strip()
        low = s.lower()
        if low.startswith(("#ifdef", "#ifndef", "#if ")):
            stack.append(s)
        elif low.startswith("#else"):
            stack[-1] = stack[-1] + "\n#else"
        elif low.startswith("#endif"):
            stack.pop()
        else:
            m = gen_bgc_state.DECL.match(line.split("!")[0])
            if m:
                out[m.group(2)] = list(stack)
    return out


def trace_module():
    out = [
        "MODULE MOD_BGC_Trace",
        "   ! @generated by oracle/scripts/gen_bgc_trace.py —— 调试专用，不进产品构建。",
        "   USE MOD_Precision",
        "   IMPLICIT NONE",
        "   integer, save :: trace_unit = -1, trace_calls = 0, trace_limit = -2",
        "   integer, save :: trace_from = 1, trace_every = huge(1)",
        "   logical, save :: trace_on = .false.",
        "CONTAINS",
        "   SUBROUTINE bgc_trace (i, ps, pe, idate, deltim, dlat, dlon, tag)",
        "   USE MOD_Vars_Global, only: nl_soil",
    ]
    modules = {}
    for name, module, _ in EXTRAS:
        if module:
            modules.setdefault(module, []).append(name)
    for module, names in modules.items():
        out.append(f"   USE {module}, only: {', '.join(names)}")
    for file, _, _ in gen_bgc_state.MODULES:
        out.append(f"   USE {file.removesuffix('.F90')}")
    out += [
        "   integer, intent(in) :: i, ps, pe, idate(3)",
        "   real(r8), intent(in) :: deltim, dlat, dlon",
        "   character(len=*), intent(in) :: tag",
        "   character(len=1024) :: path, calls",
        "   character(len=32) :: label",
        "   integer :: st",
        "      IF (trace_limit == -2) THEN",
        "         CALL get_environment_variable('COLM_BGC_TRACE', path, status=st)",
        "         IF (st /= 0) THEN",
        "            trace_limit = -1",
        "         ELSE",
        "            trace_limit = 1",
        "            CALL get_environment_variable('COLM_BGC_TRACE_CALLS', calls, status=st)",
        "            IF (st == 0) read(calls,*) trace_limit",
        "            CALL get_environment_variable('COLM_BGC_TRACE_FROM', calls, status=st)",
        "            IF (st == 0) read(calls,*) trace_from",
        "            CALL get_environment_variable('COLM_BGC_TRACE_EVERY', calls, status=st)",
        "            IF (st == 0) read(calls,*) trace_every",
        "            open(newunit=trace_unit, file=trim(path), access='stream', form='unformatted', status='replace')",
        "         ENDIF",
        "      ENDIF",
        "      IF (trace_limit < 0) RETURN",
        "      IF (tag == 'begin') THEN",
        "         trace_calls = trace_calls + 1",
        "         trace_on = trace_calls >= trace_from .and. mod(trace_calls - trace_from, trace_every) < trace_limit",
        "      ENDIF",
        "      IF (.not. trace_on) RETURN",
        "      label = tag",
        "      write(trace_unit) label",
    ]
    for name, _, kind in EXTRAS:
        emit_value(out, name, extra_slice(name, kind), "f64")
    out.append("      write(trace_unit) 0_4  ! 物理量段结束")
    for file, _, _ in gen_bgc_state.MODULES:
        cond = guards(gen_bgc_state.BGC / file)
        for name, kind, shape, _ in gen_bgc_state.parse(gen_bgc_state.BGC / file):
            expr = field_slice(name, shape)
            opened = cond.get(name, [])
            if len(opened) > 1 or any("#else" in c for c in opened):
                sys.exit(f"{name}: only a single #ifdef around a declaration is handled")
            for c in opened:
                out.append(c)
            out.append(f"      IF (allocated({name})) THEN")
            emit_value(out, name, expr, kind)
            out.append("      ELSE")
            emit_empty(out, name)
            out.append("      ENDIF")
            if opened:
                # 未声明（宏关闭）时也写 n=0，保证字段序号两边一致。
                out.append("#else")
                emit_empty(out, name)
                out.append("#endif")
    out += ["      write(trace_unit) 0_4  ! 记录结束", "      flush(trace_unit)", "   END SUBROUTINE bgc_trace", "END MODULE MOD_BGC_Trace", ""]
    return out


def instrument(driver):
    out = []
    phase = re.compile(r"CALL\s+CNPhenology\(.*phase\s*=\s*(\d)\)", re.I)
    call = re.compile(r"^\s*(?:IF\s*\(.*\)\s*)?CALL\s+(\w+)", re.I)
    in_body = False
    for line in driver.splitlines():
        code = line.split("!")[0]
        out.append(line)
        if re.match(r"\s*pe\s*=\s*patch_pft_e\(i\)", code):
            in_body = True
            out.append("      CALL bgc_trace(i, ps, pe, idate, deltim, dlat, dlon, 'begin')")
            continue
        if re.match(r"\s*IMPLICIT\s+NONE", code, re.I):
            out.insert(len(out) - 1, "   USE MOD_BGC_Trace, only: bgc_trace")
            continue
        if not in_body:
            continue
        if re.match(r"\s*END\s+SUBROUTINE", code, re.I):
            out.insert(len(out) - 1, "      CALL bgc_trace(i, ps, pe, idate, deltim, dlat, dlon, 'end')")
            continue
        m = call.match(code)
        if m:
            tag = m.group(1)
            p = phase.search(code)
            if p:
                tag += p.group(1)
            out.append(f"      CALL bgc_trace(i, ps, pe, idate, deltim, dlat, dlon, '{tag}')")
    return out


def main():
    driver = Path(sys.argv[1]).read_text()
    lines = trace_module() + instrument(driver)
    Path(sys.argv[2]).write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
