#!/usr/bin/env python3
"""Real WebView -> Tauri -> science processes multi-site smoke test.

Requires a running --features e2e-driver app in an isolated test HOME.
No IPC command starts/creates a simulation: all actions use rendered controls.
Screenshots, application lifetime, Xvfb and build provenance belong to the job wrapper.
"""
import argparse
import ast
import datetime as dt
import json
import os
import shutil
import subprocess
from pathlib import Path
import time
import urllib.request


def prelude():
    """Reuse the real driver's DOM helpers without executing its HTTP client."""
    tree = ast.parse(Path(__file__).with_name('gui-e2e.py').read_text())
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'PRE' for t in node.targets):
            return ast.literal_eval(node.value)
    raise RuntimeError('gui-e2e.py PRE helper protocol is missing')


HELPERS = r'''
const field = (names, root=document) => {
  const label = $$('.field-label', root).find(e => names.includes(txt(e)));
  if (!label) throw Error('Field missing: '+names.join('/'));
  const control = label.htmlFor ? document.getElementById(label.htmlFor) : $('input,select,textarea',label.parentElement);
  if (!control || control.disabled) throw Error('Field unavailable: '+names.join('/'));
  return control;
};
const press = (names, root=document) => {
  const choices = $$('button',root).filter(vis).filter(e => names.some(n=>txt(e)===n || txt(e).startsWith(n)));
  choices.sort((a,b)=>txt(a).length-txt(b).length);
  const control = choices[0];
  if (!control || control.disabled) throw Error('Button unavailable: '+names.join('/'));
  control.click();
};
const stationSection = name => $$('section').find(e => $('h3',e) && txt($('h3',e))===name);
const snapshot = () => ({
  text: main(),
  browserLanguage: navigator.language,
  documentLanguage: document.documentElement.lang,
  headings: $$('h1,h2,h3,h4').filter(vis).map(txt),
  buttons: buttons(),
  fields: $$('.field-label').filter(vis).map(label => {
    const el=label.htmlFor?document.getElementById(label.htmlFor):$('input,select,textarea',label.parentElement);
    return {label:txt(label), value:el?.value, disabled:el?.disabled};
  }),
  stations: $$('.simulation-monitor article').map(e=>({name:e.getAttribute('aria-label'),text:txt(e),
    active:!e.closest('details')&&/^(正在启动|运行中|Starting|Running)(?:\s|$)/.test(txt($('p',e))),
    progress:$$('progress',e).map(p=>({value:p.value,max:p.max}))})),
  charts: $$('.results-page .uplot').map(e=>({width:e.clientWidth,height:e.clientHeight,canvas:$$('canvas',e).length})),
  selectedStation: (()=>{try{return field(['站点','Station']).value}catch{return null}})(),
});
'''

# Observation only: every call is forwarded unchanged to the original real Tauri invoke.
OBSERVE = r'''
if(window.__multisiteQA) throw Error('A QA recorder is already installed; use a fresh app');
const q=window.__multisiteQA={events:[],ipc:[],errors:[],stops:[]};
for(const name of ['run://started','run://stage','run://progress','run://done'])
  q.stops.push(await window.__TAURI__.event.listen(name,e=>q.events.push({event:name,payload:e.payload,time:Date.now()})));
window.addEventListener('error',e=>q.errors.push(String(e.message)));
window.addEventListener('unhandledrejection',e=>q.errors.push(String(e.reason)));
const core=window.__TAURI__.core, original=core.invoke;
q.originalInvoke=original;
const observedInvoke=async function(command,args){
  const observed=['run_batch','run_case','cancel_runs','model_case_runs','history_catalog','history_list','history_detail','series'].includes(command);
  const record={command,args,time:Date.now()};
  if(observed) q.ipc.push(record);
  try {
    const answer=await original.apply(core,arguments);
    if(observed){
      let value=answer; if(typeof value==='string'){try{value=JSON.parse(value)}catch{}}
      record.ok=true;
      if(command==='series') record.result={n:value.n,timeCount:value.time?.length,
        first:value.time?.[0],last:value.time?.at(-1),
        variables:Object.fromEntries(Object.entries(value.vars||{}).map(([k,v])=>[k,{count:v.length,finite:v.filter(x=>typeof x==='number'&&Number.isFinite(x)).length}]))};
      else record.result=value;
    }
    return answer;
  }catch(e){if(observed){record.ok=false;record.error=String(e)}throw e;}
};
window.__TAURI__={...window.__TAURI__,core:{...core,invoke:observedInvoke}};
if(window.__TAURI__.core.invoke!==observedInvoke)throw Error('Cannot install IPC observation');
return true;
'''


def site_spec(value):
    fields = value.split(',')
    if len(fields) != 3:
        raise argparse.ArgumentTypeError('Use NAME,YYYY-MM-DD,YYYY-MM-DD')
    name, start, end = fields
    if not name or any(c in name for c in '/\\') or name in ('.', '..'):
        raise argparse.ArgumentTypeError('A site name must be one directory component')
    try:
        if dt.date.fromisoformat(start) >= dt.date.fromisoformat(end):
            raise ValueError('end must follow start')
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from error
    return {'name': name, 'start': start, 'end': end}


def validate_events(events, directories, run_id):
    summary = {}
    for directory in directories:
        own = [e for e in events if e.get('payload', {}).get('source_case', e.get('payload', {}).get('case')) == directory]
        done = [e['payload'] for e in own if e['event'] == 'run://done']
        if any(e['payload'].get('run_id') != run_id for e in own):
            raise AssertionError(f'{directory}: event belongs to another batch')
        stages = {e['payload'].get('stage') for e in own if e['event'] == 'run://stage'}
        if not done:
            raise AssertionError(f'{directory}: no real completion event')
        if done[-1].get('cancelled') or done[-1].get('code') != 0:
            raise AssertionError(f'{directory}: run failed: {done[-1]}')
        missing = {'mksrfdata', 'mkinidata', 'colm'} - stages
        if missing:
            raise AssertionError(f'{directory}: missing stage evidence {sorted(missing)}')
        finished = [e for e in own if e['event'] == 'run://done'][-1]
        started = [e for e in own if e['event'] == 'run://started'
                   and e['payload'].get('run_id') == finished['payload'].get('run_id')]
        if len(started) != 1 or not isinstance(started[0].get('time'), (int, float)) or not isinstance(finished.get('time'), (int, float)):
            raise AssertionError(f'{directory}: missing or ambiguous timed start/completion pair')
        interval = {'started_ms': started[0]['time'], 'finished_ms': finished['time']}
        if interval['started_ms'] >= interval['finished_ms']:
            raise AssertionError(f'{directory}: invalid execution interval')
        summary[directory] = {'stages': sorted(stages), 'done': done[-1], 'events': len(own), **interval}
    overlaps = []
    for index, first in enumerate(directories):
        for second in directories[index+1:]:
            begin = max(summary[first]['started_ms'], summary[second]['started_ms'])
            end = min(summary[first]['finished_ms'], summary[second]['finished_ms'])
            if end > begin:
                overlaps.append({'stations': [first, second], 'start_ms': begin, 'end_ms': end, 'duration_ms': end-begin})
    if not overlaps:
        raise AssertionError('No two stations actually overlapped; maxConcurrent alone is not concurrency evidence')
    return {'stations': summary, 'overlap_intervals': overlaps}


def validate_stopped(events, directories, run_id):
    result = {}
    for directory in directories:
        own = [e for e in events if e['payload'].get('run_id') == run_id
               and e['payload'].get('source_case', e['payload'].get('case')) == directory]
        started = [e for e in own if e['event'] == 'run://started']
        done = [e for e in own if e['event'] == 'run://done']
        assert len(started) == 1 and len(done) == 1, f'{directory}: missing actual start/exit'
        assert done[0]['payload'].get('cancelled') is True, f'{directory}: completed instead of stopping'
        assert done[0]['time'] >= started[0]['time'], f'{directory}: exit preceded start'
        result[directory] = done[0]
    return result


def validate_history_groups(page, expected, directories):
    """A fresh isolated GUI history must paginate runs, never station operations."""
    assert page['total'] == len(expected), f"History total counts station operations: {page['total']}"
    assert len(page['items']) == len(expected), 'Expected one visible row per actual run'
    groups = {}
    ids = set()
    for item in page['items']:
        group = item.get('run_group')
        assert group and group['run_id'] in expected, 'Missing or unrelated run group'
        run_id = group['run_id']
        assert run_id not in groups, 'Duplicate history rows for the same run'
        assert item['status'] == expected[run_id], 'Incorrect aggregate run status'
        members = group['members']
        assert len(members) == len(directories), 'Incorrect station count'
        assert {m['case_path'] for m in members} == set(directories), 'Wrong group station sources'
        for member in members:
            assert member['status'] == expected[run_id], 'Incorrect station terminal status'
            assert member.get('detail', {}).get('run_id') == run_id, 'Station belongs to another run'
            assert member['operation_id'] not in ids, 'Station operation repeated across groups'
            ids.add(member['operation_id'])
        groups[run_id] = item
    assert set(groups) == set(expected), 'Missing stopped or restarted run'
    return groups


class Driver:
    def __init__(self, port):
        self.base = f'http://127.0.0.1:{port}'
        self.helpers = prelude() + HELPERS

    def request(self, endpoint, body='', timeout=15):
        request = urllib.request.Request(self.base+endpoint, data=body.encode(), method='POST',
                                         headers={'X-E2E-Timeout': str(timeout)})
        with urllib.request.urlopen(request, timeout=timeout+10) as response:
            result = json.load(response)
        if not result.get('ok'):
            raise RuntimeError(result.get('value', result))
        return result.get('value')

    def js(self, body):
        return self.request('/eval', self.helpers+body)

    def wait(self, condition, timeout=60):
        until = time.monotonic()+timeout
        while time.monotonic() < until:
            result = self.js(condition)
            if result:
                return result
            time.sleep(.25)
        raise TimeoutError(condition)

    def press(self, *names, root='document'):
        self.wait('return $$('+json.dumps('button')+','+root+').some(e=>vis(e)&&!e.disabled&&'+
                  json.dumps(names)+'.some(n=>txt(e)===n||txt(e).startsWith(n)));')
        self.js('press('+json.dumps(names)+','+root+'); return true;')
        time.sleep(.2)

    def fill(self, names, value, root='document'):
        self.wait('try {field('+json.dumps(names)+','+root+');return true}catch{return false}')
        self.js('setVal(field('+json.dumps(names)+','+root+'),'+json.dumps(value)+'); return true;')
        time.sleep(.15)

    def pick(self, names, value):
        self.wait('try {field('+json.dumps(names)+');return true}catch{return false}')
        self.request('/pick', str(value))
        self.js('const el=field('+json.dumps(names)+'); const b=$("button",el.closest(".field")); '
                'if(!b||b.disabled)throw Error("Browse unavailable"); b.click(); return true;')
        self.wait('try{return field('+json.dumps(names)+').value==='+json.dumps(str(value))+';}catch{return false;}')


def run(args):
    output = Path(args.report_dir).resolve()
    output.mkdir(parents=True, exist_ok=True)
    project = Path(args.project_root).resolve()/args.name
    if project.exists():
        raise RuntimeError(f'Refusing to reuse existing project: {project}')
    data = Path(args.data_root).resolve()
    for folder in ['Sitedata', 'Forcing']:
        if not (data/folder).is_dir():
            raise RuntimeError(f'Missing input directory: {data/folder}')
    names = [site['name'] for site in args.site]
    if len(names) < 2 or len(names) != len(set(names)):
        raise RuntimeError('Specify at least two distinct --site values')
    directories = [str(project/name) for name in names]
    driver = Driver(args.port)
    report = {'success': False, 'project': str(project), 'sites': args.site, 'snapshots': []}

    def checkpoint(name):
        snapshot = driver.js('return snapshot();')
        snapshot['screenshot'] = {'status': 'not-captured', 'reason': 'Requires COLM_E2E_TEST_DISPLAY=1, DISPLAY and ImageMagick import', 'visually_reviewed': False}
        if os.environ.get('COLM_E2E_TEST_DISPLAY') == '1' and os.environ.get('DISPLAY') and shutil.which('import'):
            try:
                capture = subprocess.run(['import', '-window', 'root', str(output/(name+'.png'))], capture_output=True, text=True, timeout=15)
                snapshot['screenshot'] = {'status': 'captured' if capture.returncode == 0 else 'failed',
                    'path': str(output/(name+'.png')), 'exit_code': capture.returncode, 'stderr': capture.stderr, 'visually_reviewed': False}
            except (OSError, subprocess.TimeoutExpired) as error:
                snapshot['screenshot'] = {'status': 'failed', 'error': str(error), 'visually_reviewed': False}
        report['snapshots'].append({'name': name, 'at': time.time(), **snapshot})
        (output/(name+'.json')).write_text(json.dumps(snapshot, ensure_ascii=False, indent=2))
        print(f'GUI E2E checkpoint: {name}', flush=True)
        return snapshot

    try:
        driver.wait('return !!window.__TAURI__?.core?.invoke && buttons().length>0;')
        driver.js(OBSERVE)
        checkpoint('00-ready')
        driver.press('＋ 新建模拟', '+ New simulation', '新建模拟', 'New simulation')
        driver.wait('return !!$$(".field-label").find(e=>["模拟名称","Simulation name"].includes(txt(e)));')
        driver.press('Rust 引擎（推荐）', 'Rust engine (recommended)')
        driver.pick(['保存到项目目录', 'Project folder'], Path(args.project_root).resolve())
        driver.fill(['模拟名称', 'Simulation name'], args.name)
        driver.press('下一步', 'Next')
        driver.press('单点', 'Single site')
        driver.press('下一步', 'Next')
        driver.press('使用自己的数据', 'Use your data')
        driver.wait('return !!$$(".field-label").find(e=>["站点数据目录","Site data folder"].includes(txt(e)));')
        assert driver.js('return field(["创建方式","Creation scope"]).value;') == 'multiple', 'Multi-site is not the default'
        driver.pick(['站点数据目录', 'Site data folder'], data/'Sitedata')
        driver.pick(['强迫场目录', 'Forcing folder'], data/'Forcing')
        if args.runtime:
            driver.pick(['Runtime 数据目录', 'Runtime data folder'], Path(args.runtime).resolve())
        driver.press('读取站点与覆盖时段', 'Read sites and coverage')
        driver.wait('return '+json.dumps(names)+'.every(n=>$$("label").some(e=>txt(e)===n&&$("input[type=checkbox]",e)));', args.timeout)
        driver.press('清空选择', 'Clear selection')
        for name in names:
            driver.js('const label=$$("label").find(e=>txt(e)==='+json.dumps(name)+'); '
                      'const el=$("input[type=checkbox]",label); '
                      'if(el.disabled)throw Error("Unavailable station");if(!el.checked)el.click();return true;')
            time.sleep(.1)
        driver.fill(['预热配置方式', 'Spinup configuration'], 'per-site')
        for site in args.site:
            root = 'stationSection('+json.dumps(site['name'])+')'
            driver.wait('return !!'+root+';')
            driver.fill(['起始日期（包含预热）', 'Start date (including spinup)'], site['start'], root)
            driver.fill(['结束日期', 'End date'], site['end'], root)
            driver.fill(['预热结束 / 正式模拟开始', 'Spinup end / formal simulation start'], '', root)
        checkpoint('01-selected-stations')
        driver.press('下一步', 'Next')
        driver.press('IGBP')
        driver.press('van Genuchten' if args.hydraulics == 'vg' else 'Campbell')
        driver.press('下一步', 'Next')
        checkpoint('02-review')
        driver.press('创建所选站点', 'Create selected sites')
        driver.wait('return !!$$(".field-label").find(e=>["同时运行站点数","Concurrent stations"].includes(txt(e)));', args.timeout)
        driver.fill(['每个算例的线程数', 'Threads per case'], '1')
        driver.fill(['同时运行站点数', 'Concurrent stations'], '2')
        assert driver.js('return !document.body.innerText.includes("高级配置（引擎与计算阶段）") && !document.body.innerText.includes("Advanced settings (engine and stages)");'), 'Multi-site advanced engine/stages settings remain visible'
        checkpoint('03-resources')
        driver.press('开始运行', 'Start run')
        driver.wait('return window.__multisiteQA.events.some(e=>e.event==="run://started");', args.timeout)
        if args.stop_and_return:
            driver.wait('return new Set(window.__multisiteQA.events.filter(e=>e.event==="run://started").map(e=>e.payload.source_case||e.payload.case)).size==='+str(len(names))+' && snapshot().stations.filter(s=>s.active).length==='+str(len(names))+';', args.timeout)
            initial = driver.js('return window.__multisiteQA.ipc.filter(x=>x.command==="run_batch");')
            assert len(initial) == 1
            stopped_id = initial[0]['args']['runId']
            assert not driver.js('return window.__multisiteQA.events.some(e=>e.event==="run://done");'), 'Stop test was too late'
            driver.press('停止运行', 'Stop run')
            driver.wait('return window.__multisiteQA.events.filter(e=>e.event==="run://done"&&e.payload.run_id==='+json.dumps(stopped_id)+').length==='+str(len(names))+';', args.timeout)
            driver.wait('return window.__multisiteQA.ipc.some(x=>x.command==="run_batch"&&x.args.runId==='+json.dumps(stopped_id)+'&&x.ok);', args.timeout)
            stopped_events = driver.js('return window.__multisiteQA.events;')
            report['stopped_batch'] = {'run_id': stopped_id, 'actual_exits': validate_stopped(stopped_events, directories, stopped_id)}
            driver.wait('return snapshot().stations.every(s=>!s.active)&&$$("button").some(e=>vis(e)&&!e.disabled&&["开始运行","Start run"].includes(txt(e)));')
            checkpoint('04a-stopped')
            driver.press('项目', 'Project', root='$(".sidebar")')
            driver.wait('return !$(".simulation-monitor") && $$("h1").some(e=>/开始你的研究|Start your research/.test(txt(e)));')
            checkpoint('04b-project-home')
            driver.press('模拟', 'Simulation', root='$(".sidebar")')
            driver.wait('return !!$(".simulation-monitor") && snapshot().stations.length==='+str(len(names))+';')
            restored = checkpoint('04c-restored-stopped-batch')
            assert {s['name'] for s in restored['stations']} == set(names)
            assert all(not s['active'] and ('已停止' in s['text'] or 'Stopped' in s['text']) for s in restored['stations']), 'Stopped batch state was lost'
            assert '高级配置（引擎与计算阶段）' not in restored['text'] and 'Advanced settings (engine and stages)' not in restored['text']
            driver.press('开始运行', 'Start run')
            driver.wait('return window.__multisiteQA.ipc.filter(x=>x.command==="run_batch").length===2;', args.timeout)
        active_submission = driver.js('return window.__multisiteQA.ipc.filter(x=>x.command==="run_batch").at(-1);')
        active_id = active_submission['args']['runId']
        driver.wait('return window.__multisiteQA.events.some(e=>e.event==="run://started"&&e.payload.run_id==='+json.dumps(active_id)+');', args.timeout)
        checkpoint('04-started')
        driver.press('历史', 'History', root='$(".sidebar")')
        checkpoint('05-history')
        driver.press('模拟', 'Simulation', root='$(".sidebar")')
        driver.wait('return !!$(".simulation-monitor");')
        returned = checkpoint('06-returned-batch')
        assert all(name in [row['name'] for row in returned['stations']] for name in names), 'Returning via sidebar lost batch stations'
        for labels, expected in [(['每个算例的线程数', 'Threads per case'], '1'), (['同时运行站点数', 'Concurrent stations'], '2')]:
            actual = driver.js('const label=$$(".field-label").find(e=>'+json.dumps(labels)+'.includes(txt(e))); return document.getElementById(label.htmlFor).value;')
            assert actual == expected, f'Resource choice lost: {labels}'
        deadline = time.monotonic()+args.timeout
        samples = []
        captured_progress = False
        while time.monotonic() < deadline:
            observed = driver.js('return {events:window.__multisiteQA.events.filter(e=>e.payload.run_id==='+json.dumps(active_id)+'),view:snapshot()};')
            samples.append({'at': time.time(), 'stations': observed['view']['stations']})
            if not captured_progress and sum(bool(station['active'] and station['progress']) for station in observed['view']['stations']) >= 2:
                driver.js('$(".simulation-monitor").scrollIntoView({block:"start"});return true;')
                time.sleep(.2)
                checkpoint('06a-concurrent-progress')
                captured_progress = True
            done = [e for e in observed['events'] if e['event']=='run://done' and e['payload'].get('source_case', e['payload'].get('case')) in directories]
            if any(e['payload'].get('code') != 0 or e['payload'].get('cancelled') for e in done):
                raise AssertionError('A station failed; see captured scientific events/logs')
            if len({e['payload'].get('source_case', e['payload'].get('case')) for e in done}) == len(names):
                break
            time.sleep(1)
        else:
            raise TimeoutError('Stations did not finish')
        report['progress_samples'] = samples
        active_samples = [sample for sample in samples if sum(bool(station.get('active')) for station in sample['stations']) >= 2]
        # Checkpoints may catch a short overlap that ended while visiting History.
        active_samples.extend({'at': item['at'], 'stations': item['stations'], 'checkpoint': item['name']}
                              for item in report['snapshots'] if sum(bool(station.get('active')) for station in item['stations']) >= 2)
        report['concurrent_dom_evidence'] = {'captured': bool(active_samples), 'samples': active_samples,
            'note': 'Two active station cards captured' if active_samples else 'DOM overlap not captured; timed real events must still prove concurrency'}
        submitted = driver.js('return window.__multisiteQA.ipc.filter(x=>x.command==="run_batch");')
        assert len(submitted) == (2 if args.stop_and_return else 1), 'Unexpected real batch submission count'
        submitted = [submitted[-1]]
        report['scientific_events'] = validate_events(observed['events'], directories, submitted[0]['args']['runId'])
        assert set(submitted[0]['args']['cases']) == set(directories), 'Submitted station set differs'
        assert submitted[0]['args']['maxConcurrent'] == 2 and submitted[0]['args']['threads'] == 1, 'Resource settings did not reach the backend'
        assert submitted[0]['args']['engine'] == 'rust', 'Expected real Rust science execution'
        checkpoint('07-completed')
        driver.press('查看站点结果', 'View station results')
        for index, (name, directory) in enumerate(zip(names, directories)):
            driver.wait('return !!$(".results-page");')
            switched = driver.js('return snapshot().selectedStation;') != directory
            cursor = driver.js('return window.__multisiteQA.ipc.length;') if switched else 0
            driver.fill(['站点', 'Station'], directory)
            driver.wait('return window.__multisiteQA.ipc.slice('+str(cursor)+').some(x=>x.command==="series"&&x.ok&&x.args.case.startsWith('+json.dumps(directory+'/')+'));', args.timeout)
            driver.wait('return snapshot().selectedStation==='+json.dumps(directory)+' && snapshot().charts.some(c=>c.width>0&&c.canvas>0);', args.timeout)
            checkpoint(f'08-result-{index}-{name}')
            evidence = driver.js('return window.__multisiteQA.ipc;')
            inventories = [x for x in evidence if x['command']=='model_case_runs' and x.get('ok') and x['args'].get('dir')==directory]
            assert inventories, f'{name}: no real run inventory'
            run_paths = {r.get('run_case') for r in inventories[-1]['result'] if r.get('run_case')}
            series = [x for x in evidence[cursor:] if x['command']=='series' and x.get('ok') and x['args'].get('case') in run_paths]
            assert series and any(v['finite']>0 for v in series[-1]['result']['variables'].values()), f'{name}: no finite real result series'
            report.setdefault('result_sources', {})[name] = {'runs': sorted(run_paths), 'series': series[-1]}
        if args.stop_and_return:
            # UI query only; inspect actual backend replies rather than fabricating history.
            driver.press('历史', 'History', root='$(".sidebar")')
            driver.fill(['类型', 'Kind'], 'run')
            history_cursor = driver.js('return window.__multisiteQA.ipc.length;')
            driver.press('查询', 'Search', root='$(".history-filters")')
            driver.wait('return window.__multisiteQA.ipc.slice('+str(history_cursor)+').some(x=>x.command==="history_list"&&x.ok&&x.args.filter.kind==="run");')
            listings = driver.js('return window.__multisiteQA.ipc.slice('+str(history_cursor)+').filter(x=>x.command==="history_list"&&x.ok&&x.args.filter.kind==="run");')
            groups = validate_history_groups(listings[-1]['result'], {
                report['stopped_batch']['run_id']: 'cancelled', active_id: 'succeeded'}, directories)
            driver.wait('return $$(".history-table tbody tr").length===2;')
            rows = driver.js('return {rows:$$(".history-table tbody tr").map(txt),pagination:txt($(".history-pagination"))};')
            for run_id in groups:
                assert sum(run_id in row for row in rows['rows']) == 1, 'Run is not represented by exactly one UI row'
            assert rows['pagination'].split('/')[1].strip().startswith('2'), 'UI pagination counts stations'
            checkpoint('09-history-run-rows')
            group_row = '$$(".history-table tbody tr").find(e=>txt(e).includes('+json.dumps(active_id)+'))'
            driver.js('press(["查看","View"],'+group_row+');return true;')
            driver.wait('return txt($(".history-inspector")).includes('+json.dumps(active_id)+');')
            checkpoint('10-history-group-members')
            # Choose the second member in actual backend order, not an assumed sort order.
            member = groups[active_id]['run_group']['members'][1]
            member_name = Path(member['case_path']).name
            detail_cursor = driver.js('return window.__multisiteQA.ipc.length;')
            driver.press(member_name, root='$(".history-inspector")')
            driver.wait('return window.__multisiteQA.ipc.slice('+str(detail_cursor)+').some(x=>x.command==="history_detail"&&x.ok&&x.args.id==='+json.dumps(member['operation_id'])+');')
            details = driver.js('return window.__multisiteQA.ipc.slice('+str(detail_cursor)+').filter(x=>x.command==="history_detail"&&x.ok);')
            operation = details[-1]['result']['operation']
            assert operation['operation_id'] == member['operation_id'] and operation['case_path'] == member['case_path'], 'Group member opened another station operation'
            driver.wait('return txt($(".history-inspector")).includes('+json.dumps(member['operation_id'])+')&&txt($(".history-inspector")).includes('+json.dumps(member['case_path'])+');')
            checkpoint('11-history-station-detail')
            driver.press('打开算例', 'Open case', root='$(".history-inspector")')
            driver.press('查看结果', 'View results')
            driver.wait('return snapshot().selectedStation==='+json.dumps(member['case_path'])+'&&snapshot().charts.some(c=>c.width>0&&c.canvas>0);', args.timeout)
            evidence = driver.js('return window.__multisiteQA.ipc;')
            sources = report['result_sources'][member_name]['runs']
            actual_series = [x for x in evidence[detail_cursor:] if x['command']=='series' and x.get('ok') and x['args'].get('case') in sources]
            assert actual_series and any(v['finite']>0 for v in actual_series[-1]['result']['variables'].values()), 'History station did not load its own real results'
            checkpoint('12-history-station-results')
            report['history_groups'] = {'listing': listings[-1], 'ui': rows,
                'selected_member': member, 'detail': details[-1], 'series': actual_series[-1]}
        errors = driver.js('return window.__multisiteQA.errors;')
        assert not errors, f'Frontend errors: {errors}'
        report['success'] = True
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        try:
            report['recording'] = driver.js('const q=window.__multisiteQA;return q?{events:q.events,ipc:q.ipc,errors:q.errors}:null;')
            checkpoint('99-final')
        except Exception as error:
            report['capture_error'] = str(error)
        (output/'gui-multisite-report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))


def self_test():
    assert 'const setVal' in prelude()
    assert site_spec('CN-Cng,2008-01-01,2008-01-03')['name'] == 'CN-Cng'
    for bad in ['bad', '../escape,2008-01-01,2008-01-03', 'A,2008-01-02,2008-01-01']:
        try:
            site_spec(bad)
        except argparse.ArgumentTypeError:
            pass
        else:
            raise AssertionError(bad)
    def station_events(name, start, end):
        return ([{'event': 'run://started', 'time': start, 'payload': {'source_case': name, 'run_id': 'batch'}}]
                + [{'event': 'run://stage', 'payload': {'source_case': name, 'run_id': 'batch', 'stage': stage}} for stage in ['mksrfdata', 'mkinidata', 'colm']]
                + [{'event': 'run://done', 'time': end, 'payload': {'source_case': name, 'run_id': 'batch', 'code': 0}}])
    paths = ['/case/A', '/case/B']
    events = station_events(paths[0], 10, 30) + station_events(paths[1], 20, 40)
    result = validate_events(events, paths, 'batch')
    assert result['stations'][paths[0]]['events'] == 5
    assert result['overlap_intervals'][0]['duration_ms'] == 10
    bad_cases = [
        (events, ['/case/A', '/case/missing']),
        ([e for e in events if e['payload'].get('stage') != 'colm'], paths),
        (station_events(paths[0], 10, 20) + station_events(paths[1], 20, 30), paths),
        ([e for e in events if e['event'] != 'run://started'], paths),
    ]
    failed = station_events(paths[0], 10, 30) + station_events(paths[1], 20, 40)
    failed[-1]['payload']['code'] = 1
    bad_cases.append((failed, paths))
    stale = station_events(paths[0], 10, 30) + station_events(paths[1], 20, 40)
    stale[1]['payload']['run_id'] = 'old-batch'
    bad_cases.append((stale, paths))
    for bad_events, targets in bad_cases:
        try:
            validate_events(bad_events, targets, 'batch')
        except AssertionError:
            pass
        else:
            raise AssertionError('Missing station/stage/start, serial execution or failed physics must not pass')
    stopped = station_events(paths[0], 10, 30) + station_events(paths[1], 20, 40)
    for e in stopped:
        if e['event'] == 'run://done': e['payload']['cancelled'] = True
    assert len(validate_stopped(stopped, paths, 'batch')) == 2
    for invalid in [events, [e for e in stopped if e['event'] != 'run://done'], stopped[:-1]]:
        try:
            validate_stopped(invalid, paths, 'batch')
        except AssertionError:
            pass
        else:
            raise AssertionError('Stop request or natural completion is not actual cancellation')
    import copy
    history = {'total': 2, 'items': [
        {'status': status, 'run_group': {'run_id': run_id, 'members': [
            {'operation_id': run_id+path, 'case_path': path, 'status': status, 'detail': {'run_id': run_id}}
            for path in paths]}}
        for run_id, status in [('stopped', 'cancelled'), ('restarted', 'succeeded')]]}
    expected = {'stopped': 'cancelled', 'restarted': 'succeeded'}
    assert len(validate_history_groups(history, expected, paths)) == 2
    invalid_histories = []
    for mutation in ['total', 'duplicate', 'status', 'source', 'run_id']:
        bad = copy.deepcopy(history)
        if mutation == 'total': bad['total'] = 4
        elif mutation == 'duplicate': bad['items'][1] = copy.deepcopy(bad['items'][0])
        else:
            member = bad['items'][1]['run_group']['members'][1]
            if mutation == 'status': member['status'] = 'running'
            elif mutation == 'source': member['case_path'] = paths[0]
            else: member['detail']['run_id'] = 'stopped'
        invalid_histories.append(bad)
    for bad in invalid_histories:
        try:
            validate_history_groups(bad, expected, paths)
        except AssertionError:
            pass
        else:
            raise AssertionError('Ungrouped, duplicate or cross-run history passed')
    print('PASS: helper loading, input validation, station/stage/completion and real overlap checks (no GUI launched)')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--stop-and-return', action='store_true', help='Stop the active batch, visit Project home, restore and restart it')
    parser.add_argument('--port', type=int, default=int(os.environ.get('COLM_E2E_PORT', '18899')))
    parser.add_argument('--data-root')
    parser.add_argument('--runtime')
    parser.add_argument('--hydraulics', choices=['vg', 'campbell'], default='vg')
    parser.add_argument('--project-root')
    parser.add_argument('--name', default='gui-multisite-smoke')
    parser.add_argument('--site', action='append', type=site_spec, default=[])
    parser.add_argument('--report-dir', default=os.environ.get('OBREF_JOB_RESULTS'))
    parser.add_argument('--timeout', type=int, default=900)
    parsed = parser.parse_args()
    if parsed.self_test:
        self_test()
    else:
        for required in ['data_root', 'project_root', 'report_dir']:
            if not getattr(parsed, required):
                parser.error('--'+required.replace('_', '-')+' is required')
        if Path(parsed.name).name != parsed.name or parsed.name in ('', '.', '..'):
            parser.error('--name must be a single directory component')
        run(parsed)
