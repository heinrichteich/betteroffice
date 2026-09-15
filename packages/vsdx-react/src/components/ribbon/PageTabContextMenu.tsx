import type { TFunction } from '@betteroffice/vsdx-i18n';
import { CommandMenu } from './CommandMenu';
import type { CommandMenuEntry } from './CommandMenu';
import type { RibbonCommandId } from './commands';

export interface PageTabContextMenuProps {
  t: TFunction;
  position: { top: number; left: number };
  onClose: () => void;
  onCloseAndFocus: () => void;
}

/** Page-tab operations offered on tab right-click, reusing ribbon commands. */
export const PAGE_TAB_CONTEXT_ENTRIES: ReadonlyArray<CommandMenuEntry> = [
  { id: 'undo', icon: 'undo' },
  { id: 'redo', icon: 'redo' },
  { id: 'addShape', icon: 'add' },
];

const PAGE_TAB_CONTEXT_DIVIDERS: ReadonlySet<RibbonCommandId> = new Set(['redo']);

/** Right-click menu for a page tab, sharing the ribbon menus' keyboard behaviour. */
export function PageTabContextMenu({ t, position, onClose, onCloseAndFocus }: PageTabContextMenuProps) {
  return <CommandMenu menuLabel={t('contextMenu.pageTabLabel')} entries={PAGE_TAB_CONTEXT_ENTRIES} position={position} dividerAfter={PAGE_TAB_CONTEXT_DIVIDERS} label={(id) => t(`ribbon.commands.${id}`)} onClose={onClose} onCloseAndFocus={onCloseAndFocus} />;
}
