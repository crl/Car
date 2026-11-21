function installPlateFilter(root, entries, esc, money) {
  const panel = root.querySelector('#importPlateFilter');
  const buttons = root.querySelectorAll('[aria-controls="importPlateFilter"]');
  const search = panel.querySelector('input');
  const choices = panel.querySelector('.route-filter-choices');
  const body = root.querySelector('tbody');
  const plates = [...new Set(entries.map(entry => entry.plate))];
  const selected = new Set(plates);

  function renderRows() {
    const all = selected.size === plates.length;
    for (const button of buttons) {
      button.textContent = all ? '车牌 ▾' : `车牌（${selected.size}） ▾`;
      button.classList.toggle('is-filtered', !all);
    }
    body.innerHTML = entries.filter(entry => selected.has(entry.plate)).map(entry =>
      `<tr><td data-label="车牌">${esc(entry.plate)}</td><td data-label="起点 → 终点">${esc(entry.start)} → ${esc(entry.end)}</td><td data-label="金额">${money(entry.cents)}</td></tr>`
    ).join('') || '<tr><td colspan="3" class="empty">没有符合筛选条件的明细。</td></tr>';
  }

  function renderChoices() {
    const all = selected.size === plates.length;
    const query = search.value.trim().toLowerCase();
    const matches = plates.filter(plate => plate.toLowerCase().includes(query));
    choices.innerHTML = `<button type="button" class="project-all ${all ? 'selected' : ''}" aria-pressed="${all}">全部车辆（${selected.size}）</button>`
      + matches.map(plate => `<label class="project-chip"><input type="checkbox" value="${esc(plate)}" ${selected.has(plate) ? 'checked' : ''}><span>${esc(plate)}</span></label>`).join('')
      + (matches.length ? '' : '<p class="help">没有匹配车辆</p>');
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

export async function openExcelImport({ service, esc, money, notify, load, bindCalendarInput, categories }) {
  let file;
  try {
    file = await service.call('pick_excel_import');
  } catch (error) {
    notify(String(error));
    return;
  }
  if (file.cancelled) return;

  const dialog = document.createElement('dialog');
  dialog.className = 'excel-import';
  dialog.setAttribute('aria-labelledby', 'excel-import-title');
  dialog.innerHTML = `
    <div class="modal-head"><h2 id="excel-import-title">批量导入</h2></div>
    <div class="modal-body">
      <form class="import-fields">
        <label>子表<select name="sheet" required><option value="">请选择子表</option>
          ${file.sheets.map(sheet => `<option>${esc(sheet)}</option>`).join('')}
        </select></label>
        <label>记账日期<input id="excelImportDate" name="date" required></label>
        <label>往来方<input name="party" placeholder="如：华通" required></label>
        <div class="import-accounting">
          <div>
            <label id="import-kind-label">收支方向</label>
            <input type="hidden" name="kind" value="收入">
            <div class="segmented" role="group" aria-labelledby="import-kind-label">
              <button type="button" data-import-kind="支出" aria-pressed="false">支出</button>
              <button type="button" data-import-kind="收入" aria-pressed="true" class="selected">收入</button>
            </div>
          </div>
          <label>收支项目<select name="category" required></select></label>
        </div>
      </form>
      <div data-preview></div>
      <p role="alert"></p>
    </div>
    <div class="modal-footer row">
      <button type="button" class="secondary" data-close>取消</button>
      <button type="button" class="secondary" data-check>预览</button>
      <button type="button" data-import disabled>确认导入</button>
    </div>`;
  const form = dialog.querySelector('form');
  const previewNode = dialog.querySelector('[data-preview]');
  const errorNode = dialog.querySelector('[role=alert]');
  const check = dialog.querySelector('[data-check]');
  const submit = dialog.querySelector('[data-import]');
  const close = dialog.querySelector('[data-close]');
  let busy = false;
  let preview = null;

  function invalidate() {
    preview = null;
    submit.disabled = true;
    previewNode.replaceChildren();
    errorNode.textContent = '';
  }

  function setKind(kind) {
    form.elements.kind.value = kind;
    for (const button of form.querySelectorAll('[data-import-kind]')) {
      const selected = button.dataset.importKind === kind;
      button.classList.toggle('selected', selected);
      button.setAttribute('aria-pressed', String(selected));
    }
    const projects = categories.filter(category => category.active && category.kind === kind);
    const preferred = kind === '收入' && projects.some(category => category.name === '运费') ? '运费' : '';
    const select = form.elements.category;
    const placeholder = projects.length ? '请选择收支项目' : '请先到收支类型添加项目';
    select.replaceChildren(new Option(placeholder, ''), ...projects.map(category => new Option(category.name, category.name)));
    select.value = preferred;
    invalidate();
  }

  async function run(commit) {
    if (busy || !form.reportValidity() || (commit && !preview)) return;
    const options = Object.fromEntries(new FormData(form));
    if (!options.date) {
      errorNode.textContent = '请选择记账日期';
      form.elements.date.focus();
      return;
    }
    if (!options.party.trim()) {
      errorNode.textContent = '请填写往来方';
      return;
    }
    const expected = commit ? preview : null;
    invalidate();
    busy = true;
    close.disabled = check.disabled = true;
    for (const field of form.elements) field.disabled = true;
    try {
      const result = await service.call('import_excel', { path: file.path, options, expected });
      if (commit) {
        dialog.close();
        try {
          await load();
          notify(`已导入 ${result.count} 笔${options.category}${result.new_cars ? `，新增 ${result.new_cars} 辆车` : ''}`);
        } catch {
          notify(`已导入 ${result.count} 笔账目，刷新失败，请重新打开账目页`);
        }
        return;
      }
      preview = result;
      const { entries, errors, new_plates, new_addresses } = result;
      previewNode.innerHTML = `
        <p>${esc(result.sheet)} · ${esc(result.date)} · ${esc(result.party)} · ${esc(result.kind)} / ${esc(result.category)}</p>
        <p>明细 ${result.details} 条 · 明细金额合计 ¥${money(result.source_total)}</p>
        <p>当前可入账 ${entries.length} 笔 · ¥${money(result.total)}
          ${result.duplicates ? ` · 重复跳过 ${result.duplicates} 条（¥${money(result.duplicate_total)}）` : ''}</p>
        ${new_addresses.length ? `<p>确认后新增 ${new_addresses.length} 个自定义地址：${new_addresses.map(esc).join('、')}</p>` : ''}
        ${new_plates.length ? `<p>确认后新增 ${new_plates.length} 辆车：${new_plates.map(esc).join('、')}</p>` : ''}
        ${errors.length ? `<ul class="import-errors">${errors.map(error => `<li>${esc(error)}</li>`).join('')}</ul>` : ''}
        ${entries.length ? `
          <button type="button" class="project-header-button mobile-project-filter" aria-haspopup="dialog" aria-expanded="false" aria-controls="importPlateFilter">车牌 ▾</button>
          <div id="importPlateFilter" class="route-filter-popover" popover role="dialog" aria-label="车牌筛选">
            <div class="filter-popover-head"><strong>筛选车牌</strong><button type="button" class="text-button" aria-label="关闭车牌筛选">×</button></div>
            <label class="help">搜索车牌<input placeholder="输入车牌关键词"></label>
            <div class="route-filter-choices"></div>
          </div>
          <div class="scroll"><table><thead><tr>
            <th><button type="button" class="project-header-button" aria-haspopup="dialog" aria-expanded="false" aria-controls="importPlateFilter">车牌 ▾</button></th>
            <th>起点 → 终点</th><th>金额</th>
          </tr></thead><tbody></tbody></table></div>` : ''}`;
      if (entries.length) installPlateFilter(previewNode, entries, esc, money);
      submit.disabled = !entries.length || !!errors.length;
    } catch (error) {
      errorNode.textContent = String(error);
    } finally {
      busy = false;
      close.disabled = check.disabled = false;
      for (const field of form.elements) field.disabled = false;
    }
  }

  form.oninput = invalidate;
  for (const button of form.querySelectorAll('[data-import-kind]')) {
    button.onclick = () => {
      if (button.dataset.importKind !== form.elements.kind.value) setKind(button.dataset.importKind);
    };
  }
  setKind('收入');
  form.onsubmit = event => {
    event.preventDefault();
    void run(false);
  };
  check.onclick = () => run(false);
  submit.onclick = () => run(true);
  close.onclick = () => dialog.close();
  dialog.addEventListener('cancel', event => { if (busy) event.preventDefault(); });
  dialog.addEventListener('close', () => {
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
