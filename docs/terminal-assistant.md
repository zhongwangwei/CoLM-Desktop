# 终端 CoLM 助手

`colm-agent chat` 可在 SSH 终端或本机命令行直接聊天，不需要打开桌面窗口。它调用与桌面版相同的助手服务和工具，配置与会话默认独立保存在 `~/.colm-agent`。

## 开始使用

先配置已存在的项目目录和你能使用的模型。模型名称与思考选项以服务商当前支持的值为准：

```sh
colm-agent config --project-root /你的项目目录 \
  --base-url https://你的服务地址/v1 --provider-id custom \
  --model 你的模型名称 --api-format chat_completions
colm-agent chat
```

`--api-format` 支持 `chat_completions`、`responses`、`anthropic`。`--thinking true|false` 与 `--reasoning-effort 值` 会交给现有服务商适配器；不同模型支持的取值不同，配置成功不表示服务商接受该组合。也可以编辑配置中的 `provider.api_options`、`max_output_tokens` 与 `timeout_seconds`。

API Key 不写入模型配置或聊天。交互式保存时使用隐藏输入：

```sh
read -r -s KEY
printf '%s\n' "$KEY" | colm-agent --key-file ~/.colm-agent/keys.json \
  --set-key https://你的服务地址/v1
unset KEY
```

上面命令适用于 Bash/Zsh，服务地址必须与模型配置一致。已有部署也可通过环境变量 `COLM_AGENT_API_KEY` 或 `--key-file 路径` 提供凭据。Key 文件使用现有明文存储机制，Unix 权限为 `0600`。

## 对话与审批

| 入口 | 用途 |
|---|---|
| `/help` | 显示命令 |
| `/new` | 新对话 |
| `/sessions` | 列出已保存会话 |
| `/resume 会话ID` | 恢复已有会话 |
| `/cancel` | 取消当前回复与本机桥接进程 |
| `/exit` | 退出助手 |

交互式写入、运行和远程提交会显示具体操作，输入 `y` 批准或 `n` 拒绝。`--message` 是单轮脚本入口，自动拒绝需审批的操作；拒绝或执行错误返回非零退出码。已有独立 `colm-mcp` 仍只提供只读工具。

```sh
colm-agent chat --message '检查当前项目的运行条件'
colm-agent --data-dir ~/.colm-agent --list-sessions
colm-agent chat --resume 已存在的会话ID
```

`--data-dir` 可隔离会话，`--config` 指定非秘密配置文件；`--cli`、`--docs-root`、`--kernel-dir` 覆盖配套程序、文档与内核位置。文件工具只访问明确配置的项目目录。相对参数路径从启动命令的当前目录解析。

## 外部助手后端

```sh
colm-agent config --backend codex
colm-agent config --backend claude_code
colm-agent config --backend opencode --model 服务商/模型
```

对应外部 CLI 必须已安装，登录和服务商连接沿用项目已有适配器。终端配置本身不会登录订阅账号。OpenCode 仍使用独立 profile，首次连接与版本要求见 [助手设计稿](design-ai-assistant.md)。切回直接 API 使用 `--backend builtin`。

## 远程开发工作区

用 `--servers /绝对路径/servers.json` 配置允许使用的服务器；文件兼容桌面端服务器字段，不能包含密码或 API Key。例如沿用已有 SSH 别名：

```json
{
  "servers": [
    {
      "host": "7920ts",
      "root": "/media/zhwei/data01/colm-workspaces",
      "auth": "config",
      "scheduler": "auto",
      "cpus": 16
    }
  ]
}
```

`auth` 支持 `config`、`key`、`password`；可填写 `username`、`port`，密钥模式使用 `identity_file`。密码模式仅在 Unix 交互终端隐藏输入，Windows 或单轮脚本应使用 SSH 配置或密钥。服务器根目录与认证方式来自此文件，模型不能指定新服务器或取得凭据。

```sh
colm-agent config --servers /绝对路径/servers.json
colm-agent chat
```

助手可查询资源以及提交、查询、取消、取回已有 P3 作业。已有作业保留提交时的连接信息。退出或 Ctrl+C 取消本机交互和桥接过程；已经提交成功的远程作业继续运行，须使用作业 ID 与 `remote_workspace_cancel` 明确停止。提交期间中断可能留下远端部分状态，恢复前先查作业记录，不能盲目重复提交。完整操作和验收限制见 [远程开发工作区](design-ai-assistant.md#p3-远程开发工作区2026-10-10)。

也可让助手“检查服务器 XXX 的 `/服务器源码目录`，更新与潜热有关的知识资料”。`remote_workspace_knowledge` 只读该目录当前源码，包括未提交改动，无须本地工作区；生成卡片按服务器与目录隔离，保留时间、哈希及历史，始终标记待核对。修改或离线后旧草稿不作为当前版本依据；知识资料更新不改变服务器代码。

## pip 分发（暂时停用）

当前暂停 wheel 构建与发布，不再提供 pip 安装作为使用入口。桌面端和原生 `colm-agent chat`、`colm-agent config` 继续可用。Python 包实现及测试保留，便于后续恢复；已有本地 wheel 和安装不自动删除。状态见 [Python 分发说明](../packaging/python/README.md)。

## 经验库

直接在对话里说“查看与运行等待有关的经验”“记住这次处理办法”或“停用经验 ID”。保存和停用会显示审批；`--message` 非交互模式不能批准写入。工具失败自动保存为候选观察，经过用户接受的处理办法才可用于后续任务参考。

记录保存在 `--data-dir` 下的 `experiences/`。同一数据目录和项目路径下，换对话、重启或切换模型都可检索；桌面与终端要共享经验需指定相同数据目录。源码内容变化后旧经验标记待核对，不继续返回旧处理办法。查看时间、证据和绑定源码可用 `experience_search`，`query="*"` 返回最近五条。经验是参考，不代替当前任务证据或授权。完整状态与范围见 [本地经验库](design-ai-assistant.md#本地经验库2026-10-10)。
