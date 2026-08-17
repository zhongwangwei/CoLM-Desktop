# colm-schema 的字段表是怎么来的

`src/generated.rs` 由 `cargo run -p xtask -- gen-schema` 从
`vendor/CoLM202X/share/MOD_Namelist.F90` 生成，**产物入库**。

## 为什么入库而不是 build.rs 现生成

上游加一个 `DEF_*` 或改一个默认值，应当是一次**在 code review 里看得见**的改动。
build.rs 会让它在某次构建之后悄悄换掉，没有人经手。

## 怎么更新

```bash
git -C vendor/CoLM202X checkout <新的 commit>
cargo run -p xtask -- gen-schema
cargo test -p colm-schema      # drift 测试确认产物与源一致
git add vendor/CoLM202X crates/colm-schema/src/generated.rs
```

## 生成器必须守住的两条

1. **作用域截断**：只扫描模块声明区与 `type ... end type`，遇到第一个
   `SUBROUTINE`（第 1132 行）就停。它之后有 8 个不含 `=` 的声明是子程序局部
   变量与哑元（`nlfile` `fexists` `ivar` `ierr` `iomesg` `set_defaults` `onoff`），
   靠 `intent(...)` 属性过滤不够，因为其中 4 个没有 intent。
2. **派生类型名到实例名的映射**在 `owner_prefix` 里手工维护。
   Fortran 的类型定义与变量声明是分开的，而 namelist 文件里出现的是变量名。
   遇到未知类型时生成器会 panic，这是有意的：宁可停下来让人补，
   也不要生成一张名字错误的表。
