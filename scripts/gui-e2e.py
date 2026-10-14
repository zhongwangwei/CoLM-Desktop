#!/usr/bin/env python3
"""驱动真实桌面窗口的端到端测试客户端（配合 `--features e2e-driver` 编出的应用）。

用法：
    COLM_E2E_PORT=18899 HOME=<隔离目录> "CoLM Desktop.app/Contents/MacOS/colm-desktop-gui" &
    python3 scripts/gui-e2e.py [--timeout S] 'JS async 函数体'      # '-' 从 stdin 读

函数体在真实 WKWebView 里执行，可 `await`，`return` 的值要能 JSON 序列化。
前面自动注入一组小工具：$ / $$ / vis / txt / byText / click / sleep / until / setVal / buttons / main。

实测踩过的坑（第 692 轮）：
- 窗口在后台被遮挡时 WebKit 会节流 setTimeout，所以 sleep 用 MessageChannel 轮询；
  但长时间忙等会和 React 调度器抢 MessageChannel，一次 eval 里不要等太久——
  点击用 `setTimeout(() => click(...), 0); return 1` 立即返回，再从外面轮询页面状态。
- HOME 要短：示例站点文件路径写进 `character(len=256)`，过长会被后端拒绝。
"""
import json, os, sys, urllib.request

args = sys.argv[1:]
timeout = 60
if args and args[0] == "--timeout":
    timeout = int(args[1]); args = args[2:]
body = sys.stdin.read() if not args or args[0] == "-" else args[0]
PRE = r"""
const $$ = (s, r=document) => Array.from(r.querySelectorAll(s));
const $ = (s, r=document) => r.querySelector(s);
const vis = el => !!(el && (el.offsetWidth || el.offsetHeight || el.getClientRects().length));
const txt = el => (el.innerText || el.textContent || '').replace(/\s+/g,' ').trim();
const byText = (t, sel='button,a,[role=button],[role=tab],[role=radio],[role=option],label,summary,li,div,span') =>
  $$(sel).filter(vis).filter(e => txt(e) === t || txt(e).startsWith(t)).sort((a,b)=>txt(a).length-txt(b).length)[0];
const click = (t, sel) => { const e = typeof t === 'string' ? byText(t, sel) : t; if (!e) throw new Error('not found: '+t); e.scrollIntoView({block:'center'}); e.click(); return txt(e).slice(0,80); };
const __tick = () => new Promise(r => { const c = new MessageChannel(); c.port1.onmessage = () => r(); c.port2.postMessage(0); });
const sleep = async ms => { const end = Date.now() + ms; while (Date.now() < end) { await __tick(); await new Promise(r => queueMicrotask(r)); } };
const until = async (f, ms=30000, step=250) => { const t0=Date.now(); while (Date.now()-t0<ms) { const v = await f(); if (v) return v; await sleep(step);} throw new Error('until timeout: '+f); };
const setVal = (el, v) => { const proto = el.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype: el.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype; Object.getOwnPropertyDescriptor(proto,'value').set.call(el, v); el.dispatchEvent(new Event('input',{bubbles:true})); el.dispatchEvent(new Event('change',{bubbles:true})); };
const buttons = () => $$('button,[role=button],[role=tab]').filter(vis).map(e => (e.disabled?'[x]':'')+txt(e).slice(0,60)).filter(Boolean);
const main = () => txt($('main') || document.body);
"""
req = urllib.request.Request(f"http://127.0.0.1:{os.environ.get('COLM_E2E_PORT', '18899')}/eval", data=(PRE + "try {\n" + body + "\n} catch (e) { throw String(e); }").encode(),
                             headers={"X-E2E-Timeout": str(timeout)}, method="POST")
res = json.load(urllib.request.urlopen(req, timeout=timeout + 10))
v = res.get("value")
print(("OK " if res.get("ok") else "ERR ") + (v if isinstance(v, str) else json.dumps(v, ensure_ascii=False, indent=1)))
sys.exit(0 if res.get("ok") else 1)
