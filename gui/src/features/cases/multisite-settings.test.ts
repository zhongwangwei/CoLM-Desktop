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
  const slots: unknown[] = [];
  let cursor = 0;
  const text: string[] = [];
  const dateInputs: {
    value: string;
    disabled: boolean;
    calendarLabel: string;
    onChange: (event: { target: { value: string } }) => void;
  }[] = [];
  const context = {
    React: {
      createElement: (_type: unknown, _props: unknown, ...children: unknown[]) => {
        if (_type === context.DateInput) dateInputs.push(_props as (typeof dateInputs)[number]);
        text.push(...children.filter((child): child is string => typeof child === 'string'));
        return null;
      },
    },
    useState: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = initial;
      return [
        slots[key],
        (value: unknown) => {
          slots[key] = value;
        },
      ];
    },
    Button: () => {},
    Field: () => {},
    Input: () => {},
    DateInput: () => {},
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
    cursor = 0;
    text.length = 0;
    dateInputs.length = 0;
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
  return { props, text, dateInputs, fill, click, render, contract: () => contract };
}

test('multi-site selection excludes incomplete data and preserves edited names when selecting all', () => {
  const page = screen();
  page.fill('multisite-select:/bad.nc', 'true');
  assert.equal(page.props.drafts.length, 0);
  page.fill('multisite-select:/a.nc', 'true');
  page.fill('multisite-mode', 'per-site');
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

test('shared mode shows station names without details or per-site editable fields', () => {
  const page = screen();
  page.click('multisite-select-all');
  assert.ok(page.text.includes('A'));
  assert.ok(page.text.includes('B'));
  assert.ok(page.text.some((value) => value.includes('Output includes the first year')));
  assert.ok(page.text.some((value) => value.includes('Forcing or coverage dates are missing')));
  assert.ok(
    !page.text.some((value) => /Natural vegetation|Coverage:|Site:|Forcing:|Save to:/.test(value)),
  );
  assert.ok(!page.text.some((value) => value.includes('/a.nc') || value.includes('/project/run')));
  assert.ok(
    !page
      .contract()
      .fields.some((field) =>
        /^multisite-(name|start|end|spinup_end|spinup_repeat):/.test(field.id),
      ),
  );
  assert.equal(
    page.contract().fields.find((field) => field.id === 'multisite-select:/bad.nc')!.disabled,
    true,
  );
});

test('shared mode reports invalid leap anniversary and preserves per-site settings while hidden', () => {
  const page = screen();
  page.click('multisite-select-all');
  page.fill('multisite-mode', 'per-site');
  page.fill('multisite-start:/a.nc', '2000-02-29');
  page.fill('multisite-spinup_end:/a.nc', '2001-03-01');
  page.fill('multisite-spinup_repeat:/a.nc', '7');
  const edited = structuredClone(page.props.drafts);
  page.fill('multisite-mode', 'first-year');
  assert.ok(page.text.some((value) => value.includes('No valid anniversary')));
  assert.ok(!page.contract().fields.some((field) => field.id === 'multisite-start:/a.nc'));
  assert.deepEqual(page.props.drafts, edited);
  page.fill('multisite-mode', 'per-site');
  assert.ok(!page.text.some((value) => value.includes('No valid anniversary')));
  assert.equal(
    page.contract().fields.find((field) => field.id === 'multisite-spinup_end:/a.nc')!.value,
    '2001-03-01',
  );
  assert.equal(page.props.drafts[0].spinup_repeat, '7');
  assert.equal(page.props.drafts[1].spinup_end, '');
});

test('unavailable stations retain their specific error and shared selection remains locked', () => {
  const page = screen();
  page.props.sites[2].problem = 'Forcing dates could not be read';
  page.props.disabled = true;
  page.render();
  assert.ok(page.text.includes('Forcing dates could not be read'));
  page.click('multisite-select-all');
  page.fill('multisite-select:/a.nc', 'true');
  page.fill('multisite-mode', 'per-site');
  page.fill('multisite-repeat', '9');
  assert.deepEqual(page.props.drafts, []);
  assert.equal(page.props.mode, 'first-year');
  assert.equal(page.props.repeat, '3');
});

test('per-site date pickers update only their station and follow disabled state', () => {
  const page = screen();
  page.click('multisite-select-all');
  page.fill('multisite-mode', 'per-site');
  assert.equal(page.dateInputs.length, 6);
  assert.ok(page.dateInputs.every((input) => !input.disabled && input.calendarLabel));
  page.dateInputs[0].onChange({ target: { value: '20000201' } });
  page.render();
  assert.equal(page.props.drafts[0].start, '20000201');
  page.dateInputs[0].onChange({ target: { value: '2000-02-01' } });
  page.render();
  assert.equal(page.props.drafts[0].start, '2000-02-01');
  assert.equal(page.props.drafts[1].start, '2000-01-01');
  page.dateInputs[2].onChange({ target: { value: '2001-02-01' } });
  page.render();
  assert.equal(page.props.drafts[0].spinup_end, '2001-02-01');
  page.props.disabled = true;
  page.render();
  assert.ok(page.dateInputs.every((input) => input.disabled));
  page.props.mode = 'first-year';
  page.render();
  assert.equal(page.dateInputs.length, 0);
});

test('many sites can be filtered by name and select-all adds only the filtered available ones', () => {
  const page = screen();
  const base = page.props.sites[0];
  page.props.sites = [
    ...page.props.sites,
    ...['US-Ha1', 'US-Var', 'AU-How', 'AU-Tum', 'DE-Hai', 'FR-Pue'].map((name) => ({
      ...base,
      name,
      site_file: `/${name}.nc`,
    })),
    { ...base, name: 'US-Bad', site_file: '/US-Bad.nc', met_file: null },
  ];
  page.render();
  assert.ok(page.text.some((value) => value.includes('0 selected · 8 available · 10 total')));
  page.fill('multisite-select:/a.nc', 'true');
  page.fill('multisite-filter', 'us-');
  assert.equal(
    page.contract().actions.find((action) => action.id === 'multisite-select-all')?.label,
    'Select all filtered',
  );
  assert.ok(page.text.includes('US-Ha1'));
  assert.ok(!page.text.includes('AU-How'));
  page.click('multisite-select-all');
  assert.deepEqual(
    page.props.drafts.map((draft) => draft.name),
    ['A', 'US-Ha1', 'US-Var'],
  );
  page.fill('multisite-filter', 'nothing');
  assert.ok(page.text.some((value) => value.includes('No site name matches')));
});
