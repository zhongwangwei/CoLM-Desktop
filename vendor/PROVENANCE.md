# `vendor/CoLM202X` 的来源

**这不再是 git submodule，是入库的源码副本。**

| | |
|---|---|
| 上游 | `https://github.com/zhongwangwei/CoLM202X.git` |
| 分出来的 commit | `2f91b435` |
| 分支 | `fix/tracer-singlepoint` |
| 入库日期 | 2026-08-20 |
| 规模 | 磁盘 709 个文件 / 约 18 MB，入库 **591 个** |

## 为什么入库的比磁盘上少 118 个

`vendor/CoLM202X/.gitignore:21` 有一条 `/tests` —— **那是上游自己的规则**，
不是我们加的。CoLM 把 `tests/`（118 个 Python 静态检查与 Fortran 测试
工具）排除在版本控制外，它们在 submodule 时期同样是未跟踪的。

保持原样：入库副本与上游的跟踪范围一致。要用那些测试就照上游的做法
另外获取。

## 为什么从 submodule 改成入库副本

要把 CoLM 的**编译期宏改成运行时开关** —— `LULC` 四种、`BGC`、`CROP`、
`URBAN_MODEL`、土壤水力二选一、调试三件套，约 1100 处条件编译，
分布在 118 个 `.F90` 文件里。

目的是让**一个二进制覆盖所有配置**：现在每个宏组合都要单独编一个内核
（一个 17 MB），而有效组合有几十上百种，随包发不可能全覆盖，
让用户自己编又要求一整套 Fortran 工具链。

在 submodule 里做这种改造，每次同步上游都要 rebase 上千处改动 ——
那不是可持续的。入库之后改动就是我们自己的文件。

## 已经带进来的两处上游修复

分出来的那个 commit 里已经含着：

1. **`fix/urban-site-fallbacks`**（PR #14）—— 城市单点算例能从站点文件
   读湖深与 LAI。两处：`readflag` 的判据写错、`LAI_year` 漏读。
2. **`fix/tracer-singlepoint`**（PR #15，`2f91b435`）—— 去掉
   `create_defineh.bash` 里那条过严的 `#error`。TRACER 的 42 个模块里
   只有 2 个需要 `GridRiverLakeFlow`，而那 2 个已经各自守着自己，
   那条 `#error` 却让**每一个单点构建**都用不了 TRACER。

两个 PR 都提在上游 fork 上，接受与否不影响这份副本。

## 要同步上游时怎么做

```bash
git clone https://github.com/CoLM-SYSU/CoLM.git /tmp/colm-upstream
diff -ru /tmp/colm-upstream vendor/CoLM202X | less
```

**逐处判断**，因为我们这边会有大量有意的改动。上面那个 commit 号是
分叉点 —— 上游从那之后的改动才需要看。

## 编译时真正被读的是哪份 `define.h`

**不是 `include/define.h`。** `oracle/scripts/build_kernel.sh` 调用
`.github/workflows/create_defineh.bash`，那个脚本第 148–236 行
**整个重写** `include/define.h`（`cat>include/define.h<<EOF`）。

入库的那份静态 `include/define.h` 从来不被编译，它和生成的那份**内容不同** ——
比如静态那份有「`URBAN_MODEL && SinglePoint` 强制 `LULC_IGBP`」，
生成的那份没有。

**改宏配置要改 `create_defineh.bash`，不是 `include/define.h`。**

（那个脚本住在 `.github/workflows/` 下但不是工作流。副作用之一是
GitHub 不许没有 `workflow` scope 的 OAuth token 推它 —— 提 PR #15 时
只能走 SSH。）
