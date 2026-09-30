// JXA: every static text in SDC's windows, to read what the window says (toasts, the CLI's output).
function run() {
  const se = Application('System Events');
  const out = [];
  for (const win of se.processes.byName('sdc').windows()) {
    for (const el of win.entireContents()) {
      try { if (el.role() === 'AXStaticText') { const v = el.value() || el.name(); if (v) out.push(String(v).slice(0, 300)); } } catch (e) {}
    }
  }
  return out.join('\n');
}
