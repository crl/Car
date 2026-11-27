import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import { JSDOM } from 'jsdom';
import { createTextMatcher } from '../text-search.js';

const source = readFileSync(new URL('../excel-import.js', import.meta.url), 'utf8').replace(/^import .*;$/gm, '');
const projects = [
  { kind: '收入', name: '运费', active: true },
  { kind: '收入', name: '其他收入', active: true },
  { kind: '支出', name: '提车费', active: true },
  { kind: '支出', name: '油费', active: true },
  { kind: '支出', name: '工资', active: false },
];

async function openImport(t, { kind = '收入', defaultCategory = '运费', requiresParty = true, categories = projects, sheets = ['甲表', '乙表'], sources } = {}) {
  const dom = new JSDOM('<!doctype html><body></body>', { runScripts: 'outside-only' });
  t.after(() => dom.window.close());
  const calls = [];
  const notifications = [];
  dom.window.createTextMatcher = createTextMatcher;
  dom.window.HTMLDialogElement.prototype.showModal = function () { this.open = true; };
  dom.window.HTMLDialogElement.prototype.close = function () {
    this.open = false;
    this.dispatchEvent(new dom.window.Event('close'));
  };
  dom.window.HTMLElement.prototype.showPopover = function () { this.setAttribute('data-open', ''); };
  dom.window.HTMLElement.prototype.hidePopover = function () { this.removeAttribute('data-open'); };
  const matches = dom.window.Element.prototype.matches;
  dom.window.Element.prototype.matches = function (selector) {
    return selector === ':popover-open' ? this.hasAttribute('data-open') : matches.call(this, selector);
  };
  const openExcelImport = vm.runInContext(source.replace('export async function', 'async function') + '\nopenExcelImport', dom.getInternalVMContext());
  const preview = {
    date: '2026-11-24', party: '测试', kind, category: defaultCategory, details: 2, source_total: 10000,
    count: 1, total: 5000, duplicates: 1, duplicate_total: 5000, new_plates: [], new_addresses: [], errors: [],
    entries: [{ sheet: '甲表', plate: '沪A12345', start: '上海', end: '昆山', cents: 5000 }],
    sheets: [
      { sheet: '甲表', details: 1, source_total: 5000, duplicates: 0, duplicate_total: 0, count: 1, total: 5000 },
      { sheet: '乙表', details: 1, source_total: 5000, duplicates: 1, duplicate_total: 5000, count: 0, total: 0 },
    ],
  };
  await openExcelImport({
    service: { call: async (command, args) => {
      if (command === 'pick_excel_import') return { path: 'test.xlsx', sheets, sources, kind, default_category: defaultCategory, requires_party: requiresParty };
      assert.equal(command, 'import_excel');
      calls.push(structuredClone(args));
      return { ...preview, party: args.options.party, kind: args.options.kind, category: args.options.category ?? args.options.sources.map(s => s.category).join('、') };
    } },
    categories,
    esc: value => String(value), money: cents => (cents / 100).toFixed(2),
    notify: message => notifications.push(message), load() {}, bindCalendarInput() {},
  });
  const dialog = dom.window.document.querySelector('dialog');
  const form = dialog.querySelector('form');
  form.elements.date.value = '2026-11-24';
  if (form.elements.party) form.elements.party.value = '测试';
  return { dialog, form, calls, preview, notifications, Event: dom.window.Event };
}

test('司机工资 can preview and confirm without a counterparty', async t => {
  const {dialog, form, calls} = await openImport(t, {
    kind: '支出', defaultCategory: '司机工资', requiresParty: false,
    categories: [{kind: '支出', name: '司机工资', active: true}],
  });
  assert.equal(form.elements.namedItem('party'), null);
  await dialog.querySelector('[data-check]').onclick();
  assert.equal(dialog.querySelector('[data-preview] p').textContent, '2026-11-24 · 支出 / 司机工资');
  await dialog.querySelector('[data-import]').onclick();
  assert.equal(calls.length, 2);
  assert.ok(calls.every(call => call.options.party === ''));
});

test('mixed payroll can select only the fee from a shared sheet', async t => {
  const sources = ['司机工资','外包服务费'].map(category => ({sheet:'十月',category,enabled:true}));
  const {dialog,form,calls,preview,Event} = await openImport(t, {kind:'支出',defaultCategory:'司机工资',requiresParty:false,sources});
  assert.equal(form.elements.category, undefined);
  assert.equal(form.elements.party, undefined);
  const [salary,fee] = form.querySelectorAll('[name="sheets"]');
  assert.ok(salary.checked && fee.checked);
  salary.checked=false;
  salary.dispatchEvent(new Event('input',{bubbles:true}));
  preview.sheets = [{sheet:'十月',category:'外包服务费',details:2,source_total:201,duplicates:0,duplicate_total:0,count:1,total:201}];
  preview.entries = [{sheet:'十月', category:'外包服务费',plate:'沪A12345',start:'',end:'',cents:201,note:'由2笔合并'}];
  await dialog.querySelector('[data-check]').onclick();
  assert.deepEqual(calls[0].options.sources,[{sheet:'十月',category:'外包服务费'}]);
  assert.match(dialog.querySelector('[data-preview]').textContent,/外包服务费 · 明细 2 条 · 可入账 1 笔（¥2.01）/);
  await dialog.querySelector('[data-import]').onclick();
  assert.deepEqual(calls[1].options,calls[0].options);
});

test('select all leaves a disabled fee project unchecked', async t => {
  const sources=[{sheet:'工资',category:'司机工资',enabled:true},{sheet:'外包',category:'外包服务费',enabled:false}];
  const {dialog,form,calls}=await openImport(t,{kind:'支出',defaultCategory:'司机工资',requiresParty:false,sources});
  form.querySelector('[data-clear-sheets]').click();
  form.querySelector('[data-select-sheets]').click();
  const [,fee]=form.querySelectorAll('[name="sheets"]');
  assert.equal(fee.checked,false);
  assert.ok(fee.disabled);
  assert.match(fee.closest('label').textContent,/请先添加并启用/);
  await dialog.querySelector('[data-check]').onclick();
  assert.deepEqual(calls[0].options.sources,[{sheet:'工资',category:'司机工资'}]);
});

test('freight previews all checked sheets and invalidates confirmation when they change', async t => {
  const { dialog, form, calls, preview, Event } = await openImport(t);
  const sheets = [...form.querySelectorAll('[name=sheets]')];
  assert.deepEqual(sheets.filter(field => field.checked).map(field => field.value), ['甲表', '乙表']);
  const check = dialog.querySelector('[data-check]');
  const submit = dialog.querySelector('[data-import]');
  await check.onclick();
  assert.deepEqual(calls[0].options.sheets, ['甲表', '乙表']);
  assert.match(dialog.querySelector('[data-preview]').textContent, /乙表[\s\S]+跳过 1 条/);
  assert.equal(submit.disabled, false);

  sheets[1].checked = false;
  sheets[1].dispatchEvent(new Event('input', { bubbles: true }));
  assert.equal(submit.disabled, true);
  assert.equal(dialog.querySelector('[data-preview]').textContent, '');
  preview.errors.push('需要核对明细');
  await check.onclick();
  assert.deepEqual(calls[1].options.sheets, ['甲表']);
  assert.equal(submit.disabled, true);

  sheets[0].checked = false;
  sheets[0].dispatchEvent(new Event('input', { bubbles: true }));
  await check.onclick();
  assert.equal(calls.length, 2);
  assert.match(dialog.querySelector('[role=alert]').textContent, /至少选择一张子表/);
});

for (const [kind, defaultCategory, alternative] of [['收入', '运费', '其他收入'], ['支出', '提车费', '油费']]) {
  test(`${defaultCategory} fixes the direction and requires a fresh preview after changing project`, async t => {
    const { dialog, form, calls, notifications, Event } = await openImport(t, { kind, defaultCategory });
    const accounting = form.querySelector('.import-accounting');
    const directions = [...accounting.querySelectorAll('.segmented button')];
    assert.deepEqual(directions.map(button => button.textContent), ['支出', '收入']);
    function assertLockedDirection() {
      for (const button of directions) {
        const selected = button.textContent === kind;
        assert.equal(button.disabled, true);
        assert.equal(button.classList.contains('selected'), selected);
        assert.equal(button.getAttribute('aria-pressed'), String(selected));
      }
    }
    assertLockedDirection();
    const select = accounting.querySelector('select[name=category]');
    assert.equal(select.value, defaultCategory);
    assert.deepEqual([...select.options].map(option => option.value), ['', defaultCategory, alternative]);

    const check = dialog.querySelector('[data-check]');
    const submit = dialog.querySelector('[data-import]');
    await check.onclick();
    assertLockedDirection();
    directions.find(button => button.textContent !== kind).click();
    assertLockedDirection();
    assert.equal(submit.disabled, false);
    select.value = alternative;
    select.dispatchEvent(new Event('input', { bubbles: true }));
    assert.equal(submit.disabled, true);
    assert.equal(dialog.querySelector('[data-preview]').textContent, '');
    await submit.onclick();
    assert.equal(calls.length, 1);
    await check.onclick();
    assert.deepEqual(calls[1].options, {
      sheets: ['甲表', '乙表'], date: '2026-11-24', party: '测试', kind, category: alternative,
    });
    assert.match(dialog.querySelector('[data-preview]').textContent, new RegExp(alternative));
    assert.equal(submit.disabled, false);
    await submit.onclick();
    assert.deepEqual(calls[2].options, calls[1].options);
    assert.equal(calls[2].expected.kind, kind);
    assert.equal(calls[2].expected.category, alternative);
    assert.deepEqual(notifications, [`已导入 1 笔${alternative}`]);
  });
}

test('an unavailable default project requires an explicit choice in the same direction', async t => {
  const categories = projects.filter(project => project.name !== '提车费');
  const { dialog, form, calls } = await openImport(t, { kind: '支出', defaultCategory: '提车费', categories });
  const select = form.elements.category;
  assert.equal(select.value, '');
  assert.equal(select.selectedOptions[0].textContent, '请选择收支项目');
  await dialog.querySelector('[data-check]').onclick();
  assert.equal(calls.length, 0);
});

test('a blocked preview opens a separate problem panel and discards its contents when the preview changes', async t => {
  const { dialog, form, preview, Event } = await openImport(t);
  const confirm = dialog.querySelector('[data-import]');
  const footer = confirm.closest('.modal-footer');
  const message = footer.querySelector('[role=alert]');
  assert.ok(message, 'the confirmation footer contains the explanation');
  preview.errors.push(...Array.from({ length: 76 }, (_, row) => `「乙表」第 ${row + 2} 行与「甲表」的金额不一致`));
  const check = dialog.querySelector('[data-check]');
  await check.onclick();
  assert.equal(confirm.disabled, true);
  assert.match(message.textContent, /76 个导入问题/);
  const details = footer.querySelector('[data-show-errors]');
  assert.equal(details.hidden, false);
  const body = dialog.querySelector('.modal-body');
  body.scrollTop = 240;
  details.click();
  const panel = dialog.querySelector('[popover=manual]');
  assert.ok(panel, 'problems open in a separate non-modal popover');
  assert.equal(panel.matches(':popover-open'), true);
  assert.equal(details.getAttribute('aria-expanded'), 'true');
  assert.equal(panel.querySelectorAll('li').length, 76);
  assert.match(panel.textContent, /「乙表」第 77 行/);
  assert.equal(body.contains(panel), false);
  assert.equal(body.scrollTop, 240);
  const search = panel.querySelector('input[type=search]');
  assert.ok(search, 'the panel can filter its problems');
  search.value = '第 77 行';
  search.dispatchEvent(new Event('input', { bubbles: true }));
  assert.equal(panel.querySelectorAll('li:not([hidden])').length, 1);
  assert.match(panel.querySelector('li:not([hidden])').textContent, /「乙表」第 77 行/);
  assert.match(panel.textContent, /匹配 1 \/ 76 条/);
  for (const query of ['yibiao 77', 'YB 77']) {
    search.value = query;
    search.dispatchEvent(new Event('input', { bubbles: true }));
    assert.equal(panel.querySelectorAll('li:not([hidden])').length, 1, query);
    assert.match(panel.querySelector('li:not([hidden])').textContent, /「乙表」第 77 行/);
  }
  search.value = '无匹配';
  search.dispatchEvent(new Event('input', { bubbles: true }));
  assert.equal(panel.querySelectorAll('li:not([hidden])').length, 0);
  assert.equal(panel.querySelector('[data-no-problems]').hidden, false);
  const cancel = new Event('cancel', { cancelable: true });
  dialog.dispatchEvent(cancel);
  assert.equal(cancel.defaultPrevented, true);
  assert.equal(dialog.open, true);
  assert.equal(panel.matches(':popover-open'), false);
  details.click();

  form.querySelector('[name=sheets]').click();
  assert.equal(panel.matches(':popover-open'), false);
  assert.equal(panel.querySelectorAll('li').length, 0);
  assert.equal(message.textContent, '');
  assert.equal(details.hidden, true);
  preview.errors.splice(0, preview.errors.length, '「甲表」第 4 行需要核对');
  await check.onclick();
  details.click();
  assert.equal(panel.querySelectorAll('li').length, 1);
  assert.equal(search.value, '');
  assert.match(panel.textContent, /「甲表」第 4 行/);

  preview.errors.length = 0;
  preview.entries.length = 0;
  preview.duplicates = preview.details;
  await check.onclick();
  assert.equal(confirm.disabled, true);
  assert.match(message.textContent, /已入账/);
  assert.equal(details.hidden, true);

  preview.duplicates = 0;
  preview.details = 0;
  await check.onclick();
  assert.equal(confirm.disabled, true);
  assert.match(message.textContent, /没有可入账明细/);
});

test('sheet search filters display while global selection also includes hidden sheets', async t => {
  const { dialog, form, calls, Event } = await openImport(t, { sheets: ['国产253', '进口689', 'Sheet1'] });
  const search = form.querySelector('input[type=search]');
  assert.ok(search, 'the sheet section has a search field');
  const sheets = [...form.querySelectorAll('[name=sheets]')];
  const checked = () => sheets.filter(sheet => sheet.checked).map(sheet => sheet.value);
  const visible = () => sheets.filter(sheet => !sheet.closest('label').hidden).map(sheet => sheet.value);
  const check = dialog.querySelector('[data-check]');
  const confirm = dialog.querySelector('[data-import]');
  await check.onclick();
  assert.equal(confirm.disabled, false);
  for (const query of ['guochan253', 'GC253', 'gc 253']) {
    search.value = query;
    search.dispatchEvent(new Event('input', { bubbles: true }));
    assert.deepEqual(visible(), ['国产253']);
    assert.deepEqual(checked(), ['国产253', '进口689', 'Sheet1']);
    assert.equal(confirm.disabled, false);
  }
  const plateSearch = dialog.querySelector('#importPlateFilter input');
  for (const query of ['HUA12345', 'ha12345']) {
    plateSearch.value = query;
    plateSearch.dispatchEvent(new Event('input', { bubbles: true }));
    assert.equal(dialog.querySelector('#importPlateFilter .project-chip').textContent, '沪A12345');
    assert.equal(dialog.querySelector('#importPlateFilter .project-chip input').checked, true);
  }
  search.value = ' sheet ';
  search.dispatchEvent(new Event('input', { bubbles: true }));
  assert.deepEqual(visible(), ['Sheet1']);
  assert.deepEqual(checked(), ['国产253', '进口689', 'Sheet1']);
  assert.equal(confirm.disabled, false);

  form.querySelector('[data-clear-sheets]').click();
  assert.deepEqual(checked(), []);
  assert.equal(confirm.disabled, true);
  form.querySelector('[data-select-sheets]').click();
  assert.deepEqual(checked(), ['国产253', '进口689', 'Sheet1']);
  await check.onclick();
  assert.deepEqual(calls.at(-1).options.sheets, ['国产253', '进口689', 'Sheet1']);

  search.value = '无匹配';
  search.dispatchEvent(new Event('input', { bubbles: true }));
  assert.deepEqual(visible(), []);
  assert.equal(form.querySelector('[data-no-sheets]').hidden, false);
});
