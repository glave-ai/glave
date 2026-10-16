import type { GlzIconData } from '@glave/glaze';
import {
  FilePenLine,
  FilePlus,
  FileText,
  FolderSearch,
  Globe,
  SquareTerminal,
  TextSearch,
  Wrench,
} from 'lucide';

const TOOL_ICONS: Readonly<Record<string, GlzIconData>> = {
  bash: SquareTerminal,
  edit: FilePenLine,
  write: FilePlus,
  read: FileText,
  search: TextSearch,
  glob: FolderSearch,
  web_fetch: Globe,
};

/** The icon for a tool: the harness's own by name, a wrench for any other. */
export function toolIcon(tool: string): GlzIconData {
  return TOOL_ICONS[tool] ?? Wrench;
}
