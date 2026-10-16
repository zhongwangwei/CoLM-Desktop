import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import type { Site } from './case-model';
import { caseDirectory } from './case-model';
import {
  firstYearCutoff,
  siteDraft,
  type MultiSiteDraft,
  type MultiSiteMode,
} from './multisite-model';
import type { PageContract } from '../../lib/guide';

function screen() {
  const good: Site = {
    name: 'A',
    site_file: '/a.nc',
    met_file: '/a-met.nc',
    obs_file: null,
    urban: false,
    crop: false,
    lon: 0,
    lat: 0,
    start: '2000-01-01',
    end: '2003-12-31',
    problem: null,
  };
  const props = {
    sites: [
      good,
      { ...good, name: 'B', site_file: '/b.nc' },
      { ...good, name: 'missing', site_file: '/bad.nc', met_file: null },
    ],
    drafts: [] as MultiSiteDraft[],
    mode: 'first-year' as MultiSiteMode,
    repeat: '3',
    projectDirectory: '/project/run',
    disabled: false,
    en: true,
    onDraftsChange: (value: MultiSiteDraft[]) => {
      props.drafts = value;
    },
    onModeChange: (value: MultiSiteMode) => {
      props.mode = value;
    },
    onRepeatChange: (value: string) => {
      props.repeat = value;
    },
  };
  let contract: PageContract;
  const text: string[] = [];
  const context = {
    React: {
      createElement: (_type: unknown, _props: unknown, ...children: unknown[]) => {
        text.push(...children.filter((child): child is string => typeof child === 'string'));
        return null;
      },
    },
    Button: () => {},
    Field: () => {},
    Input: () => {},
    Notice: () => {},
    Select: () => {},
    usePageContract: (value: PageContract) => {
      contract = value;
    },
    firstYearCutoff,
    siteDraft,
    caseDirectory,
  };
  const source = readFileSync(new URL('./MultiSiteSettings.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Page = new Function(...Object.keys(context), `${code}; return MultiSiteSettings;`)(
    ...Object.values(context),
  );
  function render() {
    text.length = 0;
    Page(props);
  }
  function fill(id: string, value: string) {
    contract.fields.find((field) => field.id === id)!.set(value);
    render();
  }
  function click(id: string) {
    contract.actions.find((action) => action.id === id)!.run();
    render();
  }
  render();
  return { props, text, fill, click, render, contract: () => contract };
}

test('multi-site selection excludes incomplete data and preserves edited names when selecting all', () => {
  const page = screen();
  page.fill('multisite-select:/bad.nc', 'true');
  assert.equal(page.props.drafts.length, 0);
  page.fill('multisite-select:/a.nc', 'true');
  page.fill('multisite-name:/a.nc', 'renamed');
  page.click('multisite-select-all');
  assert.deepEqual(
    page.props.drafts.map((draft) => draft.name),
    ['renamed', 'B'],
  );
  assert.ok(page.text.includes('/project/run/renamed'));
  assert.ok(!page.contract().actions.some((action) => /next|create/.test(action.id)));
  page.props.disabled = true;
  page.render();
  page.click('multisite-clear');
  page.fill('multisite-name:/a.nc', 'blocked');
  assert.equal(page.props.drafts[0].name, 'renamed');
});

test('shared first-year cutoff is read-only, rejects leap anniversary without crashing, and allows per-site recovery', () => {
  const page = screen();
  page.click('multisite-select-all');
  assert.equal(
    page.contract().fields.find((field) => field.id === 'multisite-spinup_end:/a.nc')!.value,
    '2001-01-01',
  );
  page.fill('multisite-spinup_end:/a.nc', '2002-01-01');
  assert.equal(page.props.drafts[0].spinup_end, '');
  page.fill('multisite-start:/a.nc', '2000-02-29');
  assert.ok(page.text.some((value) => value.includes('No valid anniversary')));
  page.fill('multisite-mode', 'per-site');
  page.fill('multisite-spinup_end:/a.nc', '2001-03-01');
  page.fill('multisite-spinup_repeat:/a.nc', '7');
  assert.equal(page.props.drafts[0].spinup_end, '2001-03-01');
  assert.equal(page.props.drafts[0].spinup_repeat, '7');
  assert.equal(page.props.drafts[1].spinup_end, '');
});
