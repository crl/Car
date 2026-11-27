const collator = new Intl.Collator('zh-Hans-CN-u-co-pinyin', {
  numeric: true,
  sensitivity: 'base',
});

export const compareFilterLabels = (a, b) => collator.compare(a, b);

export function sortFilterOptions(choices, onSort) {
  const panel = choices.closest('[popover]');
  let toggle = panel.querySelector('.filter-option-sort');
  if (!toggle) {
    toggle = panel.ownerDocument.createElement('button');
    toggle.type = 'button';
    toggle.className = 'text-button filter-option-sort';
    toggle.innerHTML = `<svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
      <rect x="2" y="2" width="20" height="20" rx="5" fill="#e9ddff"/>
      <rect x="5" y="5" width="14" height="14" rx="3" fill="#fff" stroke="#d4c2ee"/>
      <path d="M8 9h3m-3 3h2.5M8 15h2" fill="none" stroke="#8056c7" stroke-width="1.4" stroke-linecap="round"/>
      <path class="filter-sort-arrow" d="M15 16V8m-2 2 2-2 2 2" fill="none" stroke="#8056c7" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>`;
    toggle.setAttribute('aria-pressed', 'false');
    const search = panel.querySelector(':scope > label');
    (search || choices).before(toggle);
    toggle.addEventListener('click', () => {
      toggle.setAttribute('aria-pressed', String(toggle.getAttribute('aria-pressed') !== 'true'));
      sortFilterOptions(choices);
      onSort?.(toggle.getAttribute('aria-pressed') === 'true');
    });
  }
  const descending = toggle.getAttribute('aria-pressed') === 'true';
  const description = descending ? '当前倒序，切换为正序' : '当前正序，切换为倒序';
  toggle.title = description;
  toggle.setAttribute('aria-label', description);

  const options = [...choices.querySelectorAll(':scope > .project-chip')];
  const label = option => option.querySelector('span').firstChild?.textContent.trim() || '';
  options.sort((a, b) => (descending ? -1 : 1) * compareFilterLabels(label(a), label(b)));
  choices.append(...options);
}
