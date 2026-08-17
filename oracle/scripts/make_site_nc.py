#!/usr/bin/env python3
"""生成增广站点文件：注入 CoLM 无条件读取但 PLUMBER2 不提供的 12 个字段。

Plan 2 中会由 colm-srfdata crate 取代本脚本；届时 colm-srfdata 必须能
逐位重现本脚本的输出。取值出处见 PROVENANCE.md。

用法: make_site_nc.py <PLUMBER2_ROOT> <输出路径>
"""
import shutil
import sys

import netCDF4 as nc
import numpy as np

SITE = "CN-Cng_2008-2009_FLUXNET2015"

# MOD_SoilColorRefl 的第 10 档（20 档中的中间值）
SOIL_ALB = {
    "soil_s_v_alb": 0.14,
    "soil_d_v_alb": 0.25,
    "soil_s_n_alb": 0.28,
    "soil_d_n_alb": 0.39,
}

# CoLM 标准 10 层土壤厚度 (m)；srfdata 只用前 8 层
DZ_SOIL = np.array(
    [0.0175, 0.0276, 0.0455, 0.0750, 0.1236, 0.2038, 0.3360, 0.5539, 0.9133, 1.5058]
)[:8]

# MOD_Initialize.F90:271 的 BVIC_USDA(0:12)
BVIC_USDA = [1.0, 0.300, 0.280, 0.250, 0.230, 0.220, 0.200, 0.180,
             0.100, 0.090, 0.150, 0.080, 0.050]


def usda_class(sand, silt, clay):
    """USDA 12 类质地。编号与 BVIC_USDA(1..12) 对齐：1=Sand … 12=Clay。"""
    if clay >= 40 and silt < 40 and sand <= 45:
        return 12, "Clay"
    if clay >= 40 and silt >= 40:
        return 11, "Silty clay"
    if clay >= 35 and sand >= 45:
        return 10, "Sandy clay"
    if 27 <= clay < 40 and 20 < sand <= 45:
        return 9, "Clay loam"
    if 27 <= clay < 40 and sand <= 20:
        return 8, "Silty clay loam"
    if 20 <= clay < 35 and silt < 28 and sand > 45:
        return 7, "Sandy clay loam"
    if silt >= 80 and clay < 12:
        return 5, "Silt"
    if silt >= 50 and (12 <= clay < 27 or clay < 12):
        return 4, "Silt loam"
    if 7 <= clay < 27 and 28 <= silt < 50 and sand <= 52:
        return 6, "Loam"
    if sand > 85 and (silt + 1.5 * clay) < 15:
        return 1, "Sand"
    if 70 <= sand <= 91 and (silt + 1.5 * clay) >= 15 and (silt + 2 * clay) < 30:
        return 2, "Loamy sand"
    return 3, "Sandy loam"


def main(plumber2_root, out_path):
    src = f"{plumber2_root}/Sitedata/{SITE}_site.nc"
    obs = f"{plumber2_root}/Observation/{SITE}_Flux.nc"
    shutil.copy(src, out_path)

    with nc.Dataset(obs) as o:
        elevation = float(np.ravel(o["elevation"][:])[0])

    d = nc.Dataset(out_path, "a")

    def put_scalar(name, value, source):
        v = d.createVariable(name, "f8")
        v[:] = value
        v.setncattr("source", source)

    put_scalar("lakedepth", 1.0, "synthesized: MOD_SingleSrfdata.F90:47 module default")
    put_scalar("elevation", elevation, f"synthesized: from {SITE}_Flux.nc elevation")
    put_scalar("elvstd", 0.0, "synthesized: MOD_SingleSrfdata.F90:88 module default")
    put_scalar("sloperatio", 0.0, "synthesized: MOD_SingleSrfdata.F90:89 module default (flat)")
    for name, val in SOIL_ALB.items():
        put_scalar(name, val, "synthesized: MOD_SoilColorRefl class L=10")

    vf_sand = np.asarray(d["soil_vf_sand"][:], dtype=float)
    vf_grav = np.asarray(d["soil_vf_gravels"][:], dtype=float)
    vf_om = np.asarray(d["soil_vf_om"][:], dtype=float)
    wf_sand = np.asarray(d["soil_wf_sand"][:], dtype=float)
    wf_grav = np.asarray(d["soil_wf_gravels"][:], dtype=float)
    omd = np.asarray(d["soil_OM_density"][:], dtype=float)
    bd = np.asarray(d["soil_BD_all"][:], dtype=float)

    wf_om = np.clip(vf_om * omd / np.where(bd > 0, bd, 1.0), 0.0, 1.0)
    vf_clay = 0.25 * np.clip(1.0 - vf_sand - vf_grav - vf_om, 0.0, 1.0)
    wf_clay = 0.25 * np.clip(1.0 - wf_sand - wf_grav - wf_om, 0.0, 1.0)

    note = ("synthesized: clay = 25% of the non-sand/gravel/OM remainder "
            "(loam 1:3 clay:silt); wf_om = vf_om * OM_density / BD_all")
    for name, arr in [("soil_vf_clay", vf_clay), ("soil_wf_clay", wf_clay),
                      ("soil_wf_om", wf_om)]:
        v = d.createVariable(name, "f8", ("soil",))
        v[:] = arr
        v.setncattr("source", note)

    # soil_texture：0-60cm 深度加权，归一化到细土后查 USDA 三角
    top = np.concatenate([[0.0], np.cumsum(DZ_SOIL)])[:8]
    bot = np.cumsum(DZ_SOIL)
    w = np.clip(np.minimum(bot, 0.60) - np.minimum(top, 0.60), 0.0, None)
    silt8 = 1.0 - wf_sand[:8] - wf_clay[:8] - wf_grav[:8] - wf_om[:8]
    tot = wf_sand[:8] + silt8 + wf_clay[:8]
    sand_pct = 100 * np.average(wf_sand[:8] / tot, weights=w)
    clay_pct = 100 * np.average(wf_clay[:8] / tot, weights=w)
    silt_pct = 100 * np.average(silt8 / tot, weights=w)
    cls, name = usda_class(sand_pct, silt_pct, clay_pct)

    v = d.createVariable("soil_texture", "i4")
    v[:] = cls
    v.setncattr(
        "source",
        f"synthesized: USDA triangle on 0-60cm depth-weighted "
        f"sand {sand_pct:.1f}% / silt {silt_pct:.1f}% / clay {clay_pct:.1f}% "
        f"-> class {cls} ({name}), BVIC {BVIC_USDA[cls]}",
    )
    d.close()
    print(f"{out_path}: soil_texture = {cls} ({name}), BVIC = {BVIC_USDA[cls]}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
