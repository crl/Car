import { sortFilterOptions, compareFilterLabels } from './filter-option-sort.js';
import { createTextMatcher } from './text-search.js';

function installPlateFilter(root, entries, showSheet, esc, money) {
  const panel = root.querySelector('#importPlateFilter');
  const buttons = root.querySelectorAll('[aria-controls="importPlateFilter"]');
  const search = panel.querySelector('input');
  const choices = panel.querySelector('.route-filter-choices');
  const body = root.querySelector('tbody');
  const plates = [...new Set(entries.map(entry => entry.plate))];
  const selected = new Set(plates);

  let descending=null;
  function renderRows() {
    const all = selected.size === plates.length;
    for (const button of buttons) {
      button.textContent = all ? '车牌 ▾' : `车牌（${selected.size}） ▾`;
      button.classList.toggle('is-filtered', !all);
    }
    const visible = entries.filter(entry => selected.has(entry.plate));
    if(descending!==null){
      visible.sort((a,b)=>(descending?-1:1)*compareFilterLabels(a.plate,b.plate));
      buttons.forEach(button=>{button.dataset.filterTableSort=descending?'↓':'↑'});
      buttons.forEach(button=>button.closest('th')?.setAttribute('aria-sort',descending?'descending':'ascending'));
    }
    body.innerHTML = visible.map(entry =>
      `<tr>${showSheet ? `<td data-label="子表">${esc(entry.sheet)}</td>` : ''}<td data-label="车牌">${esc(entry.plate)}</td><td data-label="起点 → 终点">${esc(entry.start)} → ${esc(entry.end)}</td><td data-label="金额">${money(entry.cents)}</td><td data-label="往来方">${esc(entry.party || '')}</td><td data-label="品牌">${esc(entry.brand || '')}</td><td data-label="备注">${esc(entry.note || '')}</td></tr>`
    ).join('') || `<tr><td colspan="${showSheet ? 7 : 6}" class="empty">没有符合筛选条件的明细。</td></tr>`;
  }

  function renderChoices() {
    const all = selected.size === plates.length;
    const matchesText = createTextMatcher(search.value);
    const matches = plates.filter(plate => matchesText(plate));
    choices.innerHTML = `<button type="button" class="project-all ${all ? 'selected' : ''}" aria-pressed="${all}">全部车辆（${selected.size}）</button>`
      + matches.map(plate => `<label class="project-chip"><input type="checkbox" value="${esc(plate)}" ${selected.has(plate) ? 'checked' : ''}><span>${esc(plate)}</span></label>`).join('')
      + (matches.length ? '' : '<p class="help">没有匹配车辆</p>');
    sortFilterOptions(choices, reverse=>{descending=reverse;renderRows()});
  }

  search.oninput = renderChoices;
  choices.onchange = event => {
    const { checked, value } = event.target;
    if (checked) selected.add(value);
    else selected.delete(value);
    const all = selected.size === plates.length;
    const allButton = choices.querySelector('button');
    allButton.textContent = `全部车辆（${selected.size}）`;
    allButton.classList.toggle('selected', all);
    allButton.setAttribute('aria-pressed', String(all));
    renderRows();
  };
  choices.onclick = event => {
    if (!event.target.matches('.project-all')) return;
    if (selected.size === plates.length) selected.clear();
    else for (const plate of plates) selected.add(plate);
    renderChoices();
    choices.querySelector('button').focus();
    renderRows();
  };
  panel.querySelector('.filter-popover-head button').onclick = () => panel.hidePopover();
  panel.addEventListener('toggle', event => {
    for (const button of buttons) button.setAttribute('aria-expanded', String(event.newState === 'open'));
  });
  for (const button of buttons) {
    button.onclick = () => {
      if (panel.matches(':popover-open')) {
        panel.hidePopover();
        return;
      }
      panel.showPopover();
      const rect = button.getBoundingClientRect();
      panel.style.left = Math.max(12, Math.min(rect.left, innerWidth - panel.offsetWidth - 12)) + 'px';
      panel.style.top = Math.max(12, Math.min(rect.bottom + 8, innerHeight - panel.offsetHeight - 12)) + 'px';
      search.focus();
    };
  }
  renderChoices();
  renderRows();
}

function filterByText(items, query) {
  const matchesText = createTextMatcher(query);
  let matches = 0;
  for (const item of items) {
    item.hidden = !matchesText(item.textContent);
    if (!item.hidden) matches++;
  }
  return matches;
}

function installImportProblems(dialog, trigger, esc) {
  const panel = dialog.querySelector('#importProblems');
  const header = panel.querySelector('.import-problems-head');
  const closeButton = panel.querySelector('[data-close-problems]');
  const search = panel.querySelector('input');
  const list = panel.querySelector('ul');
  const count = panel.querySelector('[data-problem-count]');
  const empty = panel.querySelector('[data-no-problems]');
  const body = dialog.querySelector('.modal-body');
  const footer = dialog.querySelector('.modal-footer');
  const gap = 12;
  let drag = null;
  let moved = false;

  function move(left, top) {
    panel.style.left = Math.max(gap, Math.min(left, innerWidth - panel.offsetWidth - gap)) + 'px';
    panel.style.top = Math.max(gap, Math.min(top, innerHeight - panel.offsetHeight - gap)) + 'px';
  }

  function keepInView() {
    move(panel.offsetLeft, panel.offsetTop);
  }

  function place() {
    if (moved) {
      keepInView();
      return;
    }
    const rect = dialog.getBoundingClientRect();
    const right = rect.right + gap;
    const left = rect.left - panel.offsetWidth - gap;
    const fitsRight = right + panel.offsetWidth + gap <= innerWidth;
    const overlaps = !fitsRight && left < gap;
    const top = overlaps ? body.getBoundingClientRect().top : rect.top;
    panel.style.removeProperty('--import-problems-max-height');
    if (overlaps) {
      const available = footer.getBoundingClientRect().top - top - gap;
      panel.style.setProperty('--import-problems-max-height', Math.max(0, available) + 'px');
    }
    move(fitsRight ? right : overlaps ? rect.right - panel.offsetWidth - gap : left, top);
  }

  function close(restoreFocus = false) {
    window.removeEventListener('resize', place);
    if (!panel.matches(':popover-open')) return false;
    if (drag && header.hasPointerCapture(drag.id)) header.releasePointerCapture(drag.id);
    drag = null;
    panel.hidePopover();
    trigger.setAttribute('aria-expanded', 'false');
    if (restoreFocus) trigger.focus({ preventScroll: true });
    return true;
  }

  function filter() {
    const matches = filterByText(list.children, search.value);
    const total = list.childElementCount;
    count.textContent = search.value.trim() ? `匹配 ${matches} / ${total} 条` : `共 ${total} 条`;
    empty.hidden = matches > 0;
    list.scrollTop = 0;
    if (panel.matches(':popover-open')) keepInView();
  }

  trigger.onclick = () => {
    if (panel.matches(':popover-open')) return;
    moved = false;
    panel.showPopover();
    place();
    window.addEventListener('resize', place);
    trigger.setAttribute('aria-expanded', 'true');
    search.focus({ preventScroll: true });
  };
  closeButton.onclick = () => close(true);
  search.oninput = filter;
  dialog.addEventListener('keydown', event => {
    if (event.key === 'Escape' && close(true)) event.preventDefault();
  });

  header.onpointerdown = event => {
    if (event.button !== 0 || drag || event.target.closest('button')) return;
    drag = { id: event.pointerId, x: event.clientX - panel.offsetLeft, y: event.clientY - panel.offsetTop };
    header.setPointerCapture(event.pointerId);
    event.preventDefault();
  };
  header.onpointermove = event => {
    if (drag?.id !== event.pointerId) return;
    moved = true;
    move(event.clientX - drag.x, event.clientY - drag.y);
  };
  header.onlostpointercapture = () => { drag = null; };

  return {
    close,
    setErrors(errors) {
      search.value = '';
      list.innerHTML = errors.map(error => `<li>${esc(error)}</li>`).join('');
      filter();
    },
  };
}

export async function openExcelImport({ service, esc, money, notify, load, bindCalendarInput, categories }) {
  let file;
  try {
    file = await service.call('pick_excel_import');
  } catch (error) {
    notify(String(error));
    return;
  }
  if (file.cancelled) return;
  const { kind, default_category: defaultCategory, requires_party: requiresParty = true } = file;
  const sources = file.sources;
  const choices = sources || file.sheets.map(sheet => ({sheet, enabled: true}));

  const dialog = document.createElement('dialog');
  dialog.className = 'excel-import';
  dialog.setAttribute('aria-labelledby', 'excel-import-title');
  dialog.innerHTML = `
    <div class="modal-head"><h2 id="excel-import-title">批量导入</h2></div>
    <div class="modal-body">
      <form class="import-fields">
        <fieldset class="import-sheets" aria-labelledby="importSheetsTitle">
          <div class="import-sheet-tools">
            <span id="importSheetsTitle" class="import-sheet-title muted">子表</span>
            <input type="search" aria-label="搜索子表" placeholder="搜索子表" data-sheet-search>
            <button type="button" class="secondary" data-select-sheets>全选</button>
            <button type="button" class="secondary" data-clear-sheets>全不选</button>
          </div>
          <div class="import-sheet-choices">
            ${choices.map(({sheet, category, enabled}, index) => `<label class="project-chip"><input type="checkbox" name="sheets" value="${sources ? index : esc(sheet)}" ${enabled ? 'checked' : 'disabled'}><span>${esc(sheet)}${sources ? ' · ' + esc(category) : ''}${enabled ? '' : '（请先添加并启用该支出项目）'}</span></label>`).join('')}
          </div>
          <p class="help" data-no-sheets hidden>没有匹配的子表</p>
        </fieldset>
        <label>记账日期<input id="excelImportDate" name="date" required></label>
        ${requiresParty ? '<label>往来方<input name="party" placeholder="如：华通" required></label>' : ''}
        <div class="import-accounting">
          <fieldset class="import-direction">
            <legend>收支方向</legend>
            <div class="segmented">
              ${['支出', '收入'].map(direction => {
                const selected = direction === kind;
                return `<button type="button" disabled class="${selected ? 'selected' : ''}" aria-pressed="${selected}">${direction}</button>`;
              }).join('')}
            </div>
          </fieldset>
          ${sources ? '' : '<label>收支项目<select name="category" required></select></label>'}
        </div>
      </form>
      <div data-preview></div>
    </div>
    <div class="modal-footer row">
      <div class="import-feedback">
        <p role="alert"></p>
        <button type="button" class="text-button" data-show-errors aria-haspopup="dialog" aria-controls="importProblems" aria-expanded="false" hidden>查看问题</button>
      </div>
      <button type="button" class="secondary" data-close>取消</button>
      <button type="button" class="secondary" data-check>预览</button>
      <button type="button" data-import disabled>确认导入</button>
    </div>
    <div id="importProblems" class="import-problems" popover="manual" role="dialog" aria-labelledby="importProblemsTitle">
      <div class="import-problems-head">
        <h3 id="importProblemsTitle">导入问题</h3>
        <span class="muted" data-problem-count aria-live="polite"></span>
        <button type="button" class="text-button" aria-label="关闭导入问题" data-close-problems>×</button>
      </div>
      <div class="import-problems-search">
        <input type="search" aria-label="搜索导入问题" placeholder="搜索子表、行号或原因">
      </div>
      <ul class="import-problems-list" aria-label="导入问题列表" tabindex="0"></ul>
      <p class="help import-problems-empty" data-no-problems hidden>没有匹配的问题</p>
    </div>`;
  const form = dialog.querySelector('form');
  const previewNode = dialog.querySelector('[data-preview]');
  const errorNode = dialog.querySelector('[role=alert]');
  const showErrors = dialog.querySelector('[data-show-errors]');
  const problems = installImportProblems(dialog, showErrors, esc);
  const sheetSearch = form.querySelector('[data-sheet-search]');
  const sheetFields = [...form.querySelectorAll('[name=sheets]')];
  const sheetLabels = sheetFields.map(field => field.closest('label'));
  const noSheets = form.querySelector('[data-no-sheets]');
  const check = dialog.querySelector('[data-check]');
  const submit = dialog.querySelector('[data-import]');
  const close = dialog.querySelector('[data-close]');
  const projects = categories.filter(category => category.active && category.kind === kind);
  const select = form.elements.category;
  const placeholder = projects.length ? '请选择收支项目' : '请先到收支类型添加项目';
  if (select) {
    select.replaceChildren(new Option(placeholder, ''), ...projects.map(category => new Option(category.name, category.name)));
    select.value = projects.some(category => category.name === defaultCategory) ? defaultCategory : '';
  }
  let busy = false;
  let preview = null;

  function invalidate() {
    problems.close();
    problems.setErrors([]);
    preview = null;
    submit.disabled = true;
    previewNode.replaceChildren();
    errorNode.textContent = '';
    showErrors.hidden = true;
  }

  function selectSheets(checked) {
    if (!sheetFields.some(field => !field.disabled && field.checked !== checked)) return;
    for (const field of sheetFields) if (!field.disabled) field.checked = checked;
    invalidate();
  }

  async function run(commit) {
    if (busy || (commit && !preview)) return;
    const expected = commit ? preview : null;
    invalidate();
    if (!form.reportValidity()) return;
    const fields = new FormData(form);
    const selected = fields.getAll('sheets');
    const options = {date: fields.get('date'), party: fields.get('party') || '', kind};
    if (sources) {
      options.sources = selected.map(index => {
        const {sheet, category} = sources[Number(index)];
        return {sheet, category};
      });
    } else {
      options.sheets = selected;
      options.category = fields.get('category');
    }
    if (!selected.length) {
      errorNode.textContent = '请至少选择一张子表';
      sheetSearch.focus();
      return;
    }
    if (!options.date) {
      errorNode.textContent = '请选择记账日期';
      form.elements.date.focus();
      return;
    }
    if (requiresParty && !options.party.trim()) {
      errorNode.textContent = '请填写往来方';
      return;
    }
    busy = true;
    close.disabled = check.disabled = true;
    const enabledFields = [...form.elements].filter(field => !field.disabled);
    for (const field of enabledFields) field.disabled = true;
    try {
      const result = await service.call('import_excel', { path: file.path, options, expected });
      if (commit) {
        dialog.close();
        try {
          await load();
          notify(`已导入 ${result.count} 笔${options.category || '支出'}${result.new_cars ? `，新增 ${result.new_cars} 辆车` : ''}`);
        } catch {
          notify(`已导入 ${result.count} 笔账目，刷新失败，请重新打开账目页`);
        }
        return;
      }
      preview = result;
      const { entries, errors, warnings = [], new_plates, new_addresses } = result;
      const showSheet = !!sources || result.sheets.length > 1;
      const projectTotals = new Map();
      if (sources) for (const sheet of result.sheets) {
        const totals = projectTotals.get(sheet.category) || {details: 0, count: 0, total: 0};
        totals.details += sheet.details;
        totals.count += sheet.count;
        totals.total += sheet.total;
        projectTotals.set(sheet.category, totals);
      }
      previewNode.innerHTML = `
        <p>${esc(result.date)} · ${result.party ? esc(result.party) + ' · ' : ''}${esc(result.kind)} / ${esc(result.category)}</p>
        <p>明细 ${result.details} 条 · 明细金额合计 ¥${money(result.source_total)}</p>
        <p>当前可入账 ${entries.length} 笔 · ¥${money(result.total)}
          ${result.duplicates ? ` · 重复跳过 ${result.duplicates} 条（¥${money(result.duplicate_total)}）` : ''}</p>
        ${[...projectTotals].map(([category, totals]) => `<p><strong>${esc(category)}</strong> · 明细 ${totals.details} 条 · 可入账 ${totals.count} 笔（¥${money(totals.total)}）</p>`).join('')}
        <ul class="import-summaries">${result.sheets.map(sheet => `<li><strong>${esc(sheet.sheet)}${sources ? ' · ' + esc(sheet.category) : ''}</strong>
          · 明细 ${sheet.details} 条（¥${money(sheet.source_total)}） · 跳过 ${sheet.duplicates} 条 · 可入账 ${sheet.count} 笔（¥${money(sheet.total)}）</li>`).join('')}</ul>
        ${new_addresses.length ? `<p>确认后新增 ${new_addresses.length} 个自定义地址：${new_addresses.map(esc).join('、')}</p>` : ''}
        ${new_plates.length ? `<p>确认后新增 ${new_plates.length} 辆车：${new_plates.map(esc).join('、')}</p>` : ''}
        ${entries.length ? `
          <button type="button" class="project-header-button mobile-project-filter" aria-haspopup="dialog" aria-expanded="false" aria-controls="importPlateFilter">车牌 ▾</button>
          <div id="importPlateFilter" class="route-filter-popover" popover role="dialog" aria-label="车牌筛选">
            <div class="filter-popover-head"><strong>筛选车牌</strong><button type="button" class="text-button" aria-label="关闭车牌筛选">×</button></div>
            <label class="help">搜索车牌<input placeholder="输入车牌关键词"></label>
            <div class="route-filter-choices"></div>
          </div>
          <div class="scroll"><table><thead><tr>
            ${showSheet ? '<th>子表</th>' : ''}
            <th><button type="button" class="project-header-button" aria-haspopup="dialog" aria-expanded="false" aria-controls="importPlateFilter">车牌 ▾</button></th>
            <th>起点 → 终点</th><th>金额</th><th>往来方</th><th>品牌</th><th>备注</th>
          </tr></thead><tbody></tbody></table></div>` : ''}`;
      if (entries.length) {
        const displayEntries = sources ? entries.map(entry => ({...entry, sheet: `${entry.sheet} · ${entry.category}`})) : entries;
        installPlateFilter(previewNode, displayEntries, showSheet, esc, money);
      }
      submit.disabled = !entries.length || !!errors.length;
      if (errors.length || warnings.length) {
        problems.setErrors([...errors, ...warnings]);
        errorNode.textContent = errors.length
          ? `有 ${errors.length} 个导入问题，核对后请重新预览。`
          : `已跳过 ${warnings.length} 行，请查看原因并核对有效明细。`;
        showErrors.hidden = false;
      } else if (!entries.length) {
        errorNode.textContent = result.duplicates ? '所选明细均已入账。' : '所选子表没有可入账明细。';
      }
    } catch (error) {
      errorNode.textContent = String(error);
    } finally {
      busy = false;
      close.disabled = check.disabled = false;
      for (const field of enabledFields) field.disabled = false;
    }
  }

  sheetSearch.oninput = () => { noSheets.hidden = filterByText(sheetLabels, sheetSearch.value) > 0; };
  sheetSearch.onkeydown = event => { if (event.key === 'Enter') event.preventDefault(); };
  form.querySelector('[data-select-sheets]').onclick = () => selectSheets(true);
  form.querySelector('[data-clear-sheets]').onclick = () => selectSheets(false);
  form.oninput = event => { if (event.target !== sheetSearch) invalidate(); };
  form.onsubmit = event => {
    event.preventDefault();
    void run(false);
  };
  check.onclick = () => run(false);
  submit.onclick = () => run(true);
  close.onclick = () => dialog.close();
  dialog.addEventListener('cancel', event => { if (problems.close(true) || busy) event.preventDefault(); });
  dialog.addEventListener('close', () => {
    problems.close();
    const calendar = dialog.querySelector('#ledgerCalendar');
    if (calendar) {
      calendar.hidePopover();
      document.body.append(calendar);
    }
    dialog.remove();
  }, { once: true });
  document.body.append(dialog);
  bindCalendarInput(form.elements.date);
  dialog.showModal();
}
