import { FilePen, FileText, Play } from 'lucide-react';

import { strings } from '../../strings';

/** The icon and the verb for a tool, by its Claude Code name or the SDC Agent's. */
export function look(name: string): { icon: typeof FilePen; verb: string } {
  const d = strings.turns.draft;

  switch (name) {
    case 'Write':
    case 'write_file':
      return { icon: FilePen, verb: d.write };
    case 'Edit':
    case 'MultiEdit':
    case 'NotebookEdit':
    case 'edit_file':
    case 'apply_patch':
      return { icon: FilePen, verb: d.edit };
    case 'Bash':
    case 'PowerShell':
    case 'run_command':
      return { icon: Play, verb: d.run };
    default:
      return { icon: FileText, verb: d.other };
  }
}
