// JXA: press the first control in SDC's window whose accessible text contains argv[0] (exact when argv[1] is "exact").
function run(argv) {
  const needle = argv[0];
  const exact = argv[1] === 'exact';
  const se = Application('System Events');
  const win = se.processes.byName('sdc').windows[0];
  const all = win.entireContents();
  for (const el of all) {
    let text = '';
    let role = '';
    try { role = el.role(); } catch (e) {}
    if (role !== 'AXButton' && role !== 'AXCheckBox' && role !== 'AXLink') continue;
    let parts = [];
    try { parts = [el.name(), el.description(), el.title && el.title()].filter(Boolean).map((t) => String(t).trim()); } catch (e) {}
    text = parts.join(' | ');
    if (exact ? parts.includes(needle) : text.includes(needle)) {
      el.actions.byName('AXPress').perform();
      return 'pressed [' + role + '] ' + text;
    }
  }
  return 'NOT FOUND: ' + needle;
}
