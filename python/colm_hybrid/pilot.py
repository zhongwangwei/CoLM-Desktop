"""land_class 插槽的试点训练：PLUMBER2 站点上学"静态特征 → Vcmax"，在留出站点上与纯物理比较。

    python -m colm_hybrid.pilot prepare  --sites sites.json --work DIR --cli colm-cli --kernel KERNEL
    python -m colm_hybrid.pilot train    --work DIR --cli colm-cli --kernel KERNEL [--generations 10] [--population 8]
    python -m colm_hybrid.pilot evaluate --work DIR --cli colm-cli --kernel KERNEL

（在 python/ 目录下运行。）

- prepare：每个站点建一个两年的算例（第一年 spin-up，第二年写 history），跑纯物理基线，记下基线指标。
- train：线性模型（标准化特征 → sigmoid → [10, 150] µmol/m²/s 的 DEF_LC_VMAX25），CMA-ES 无梯度优化；
  损失是训练站点上 1 - KGE(Qle) 的平均。每个候选导出成 ONNX、写进各算例的 hybrid.toml，只重跑 colm 段
  （前处理靠指纹跳过）。站点之间并行。
- evaluate：最优模型在所有站点上跑一遍，与基线比较。

站点文件 sites.json：[{"name", "split": "train"|"test", "site", "met", "obs", "year"}, ...]。
"""

import argparse
import concurrent.futures as futures
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

import numpy as np

from .features import read_features
from .onnx_export import export_mlp

FEATURES = ["patchlatr", "elvmean", "htop", "vf_clay[1]", "vf_sand[1]", "OM_density[1]"]
OUTPUT = "DEF_LC_VMAX25"
RANGE = (10.0, 150.0)
FLUX = "Qle"


def run(args, check=True):
    result = subprocess.run(args, capture_output=True, text=True)
    if check and result.returncode != 0:
        raise RuntimeError(f"{' '.join(map(str, args))}\n{result.stdout[-2000:]}\n{result.stderr[-2000:]}")
    return result


def metrics(cli, case, obs):
    out = run([cli, "metrics", case, "--obs", obs, "--json", "1", "--summary-only", "1"]).stdout
    return {row["name"]: row for row in json.loads(out)}


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_slot(case: Path, layers, norm_path: Path, label: str):
    """把模型与配置写进算例目录；返回 hybrid.toml 路径。"""
    models = case / "models"
    models.mkdir(exist_ok=True)
    model = models / f"{label}.onnx"
    export_mlp(layers, str(model))
    norm = models / "norm.json"
    norm.write_text(norm_path.read_text())
    features = ", ".join(f'"{name}"' for name in FEATURES)
    text = (
        "[[slot]]\n"
        'name = "land_class"\n'
        'kind = "param"\n'
        f'model = "models/{model.name}"\n'
        f'sha256 = "{sha256(model)}"\n'
        f"features = [{features}]\n"
        'normalize = "models/norm.json"\n'
        f'outputs = [{{ name = "{OUTPUT}", range = [{RANGE[0]}, {RANGE[1]}], transform = "sigmoid" }}]\n'
    )
    path = case / "hybrid.toml"
    path.write_text(text)
    return path


def linear_layers(theta):
    theta = np.asarray(theta, dtype=np.float64)
    return [(theta[:-1].reshape(-1, 1), theta[-1:])]


def evaluate_candidate(work: Path, cli, kernel, sites, layers, label, jobs):
    norm = work / "norm.json"

    def one(site):
        case = work / "cases" / site["name"]
        write_slot(case, layers, norm, label)
        run([cli, "run", str(case), "--kernel", kernel, "--stage", "colm"])
        return site["name"], metrics(cli, str(case), site["obs"])

    with futures.ThreadPoolExecutor(jobs) as pool:
        return dict(pool.map(one, sites))


def remove_slot(case: Path):
    for name in ["hybrid.toml"]:
        (case / name).unlink(missing_ok=True)


def prepare(a):
    work = Path(a.work)
    (work / "cases").mkdir(parents=True, exist_ok=True)
    sites = json.load(open(a.sites))

    def one(site):
        case = work / "cases" / site["name"]
        if not (case / "case.nml").exists():
            y = site["year"]
            run([a.cli, "new", "--site", site["site"], "--out", str(case), "--name", site["name"],
                 "--start", f"{y}-01-01", "--end", f"{y + 1}-12-31", "--met", site["met"],
                 "--mode", "igbp", "--spinup-years", "1", "--spinup-repeat", "1"])
        remove_slot(case)
        run([a.cli, "run", str(case), "--kernel", a.kernel])
        return site["name"], metrics(a.cli, str(case), site["obs"])

    with futures.ThreadPoolExecutor(a.jobs) as pool:
        baseline = dict(pool.map(one, sites))
    json.dump(sites, open(work / "sites.json", "w"), indent=1)
    json.dump(baseline, open(work / "baseline.json", "w"), indent=1)
    features = {s["name"]: read_features(work / "cases" / s["name"], FEATURES) for s in sites}
    json.dump(features, open(work / "features.json", "w"), indent=1)
    train = np.array([features[s["name"]] for s in sites if s["split"] == "train"])
    norm = {"mean": train.mean(0).tolist(), "std": np.where(train.std(0) > 0, train.std(0), 1.0).tolist()}
    json.dump(norm, open(work / "norm.json", "w"), indent=1)
    for s in sites:
        print(f"{s['name']:8s} {s['split']:5s} KGE({FLUX}) = {baseline[s['name']][FLUX]['kge']:.3f}")


def loss_of(result, sites):
    return float(np.mean([1.0 - result[s["name"]][FLUX]["kge"] for s in sites]))


def train(a):
    import cma

    work = Path(a.work)
    sites = [s for s in json.load(open(work / "sites.json")) if s["split"] == "train"]
    baseline = json.load(open(work / "baseline.json"))
    print(f"baseline train loss {np.mean([1 - baseline[s['name']][FLUX]['kge'] for s in sites]):.4f}", flush=True)
    theta0 = np.zeros(len(FEATURES) + 1)
    es = cma.CMAEvolutionStrategy(theta0, 1.0, {"popsize": a.population, "maxiter": a.generations,
                                                 "seed": 1, "verbose": -9})
    history = []
    best = (np.inf, theta0)
    generation = 0
    while not es.stop():
        candidates = es.ask()
        losses = []
        for index, theta in enumerate(candidates):
            result = evaluate_candidate(work, a.cli, a.kernel, sites, linear_layers(theta),
                                        f"g{generation}c{index}", a.jobs)
            loss = loss_of(result, sites)
            losses.append(loss)
            if loss < best[0]:
                best = (loss, np.asarray(theta))
        es.tell(candidates, losses)
        history.append({"generation": generation, "losses": losses, "best": best[0]})
        print(f"generation {generation}: min {min(losses):.4f} mean {np.mean(losses):.4f} best {best[0]:.4f}", flush=True)
        json.dump({"theta": best[1].tolist(), "loss": best[0], "features": FEATURES, "history": history},
                  open(work / "best.json", "w"), indent=1)
        generation += 1


def evaluate(a):
    work = Path(a.work)
    sites = json.load(open(work / "sites.json"))
    baseline = json.load(open(work / "baseline.json"))
    best = json.load(open(work / "best.json"))
    layers = linear_layers(best["theta"])
    result = evaluate_candidate(work, a.cli, a.kernel, sites, layers, "best", a.jobs)
    json.dump(result, open(work / "hybrid_metrics.json", "w"), indent=1)
    rows = []
    for split in ["train", "test"]:
        names = [s["name"] for s in sites if s["split"] == split]
        for name in names:
            b, h = baseline[name], result[name]
            rows.append((split, name, b[FLUX]["kge"], h[FLUX]["kge"], b["Qh"]["kge"], h["Qh"]["kge"]))
        bk = np.mean([baseline[n][FLUX]["kge"] for n in names])
        hk = np.mean([result[n][FLUX]["kge"] for n in names])
        print(f"{split}: mean KGE({FLUX}) physics {bk:.3f} -> hybrid {hk:.3f}")
    print(f"{'split':5s} {'site':8s} {'Qle phys':>9s} {'Qle hyb':>9s} {'Qh phys':>9s} {'Qh hyb':>9s}")
    for row in rows:
        print(f"{row[0]:5s} {row[1]:8s} {row[2]:9.3f} {row[3]:9.3f} {row[4]:9.3f} {row[5]:9.3f}")


def main(argv=None):
    p = argparse.ArgumentParser()
    p.add_argument("command", choices=["prepare", "train", "evaluate"])
    p.add_argument("--sites")
    p.add_argument("--work", required=True)
    p.add_argument("--cli", required=True)
    p.add_argument("--kernel", required=True)
    p.add_argument("--jobs", type=int, default=os.cpu_count())
    p.add_argument("--generations", type=int, default=10)
    p.add_argument("--population", type=int, default=8)
    a = p.parse_args(argv)
    {"prepare": prepare, "train": train, "evaluate": evaluate}[a.command](a)


if __name__ == "__main__":
    sys.exit(main())
