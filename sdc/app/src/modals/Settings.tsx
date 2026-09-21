import { version as APP_VERSION } from '../../../package.json';
import { SDCP_VERSION } from '../../../protocol/types';
import { useState, type ReactNode } from 'react';
import {
  Bell,
  DatabaseBackup,
  Info,
  Keyboard,
  Palette as PaletteIcon,
  Shield,
  SlidersHorizontal,
  Stethoscope,
  type LucideIcon,
} from 'lucide-react';

import { commandsByGroup } from '../commands/registry';
import { setTheme } from '../lib/theme';
import { strings } from '../strings';
import { useLayoutStore } from '../store/layout';
import { localHost } from '../store/reducer';
import { useAppStore } from '../store/store';
import { toast } from '../store/toast';
import { useOverlayStore, type SettingsTab } from '../store/overlays';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { Modal } from './Modal';

/**
 * `#settingsBd` - Settings, seven tabs (spec section 9.11).
 *
 * The tab bodies are *data*: `strings.settings.<tab>` holds the groups and rows, and this component
 * is one renderer for all of them. That is why the seven tabs are the spec's seven tabs and not a
 * paraphrase - the copy and the order live in one table, and a row is added by adding a row.
 *
 * Three tabs are not pure tables and are handled by name:
 *
 *   Keymap    reads `commandsByGroup()` - the same registry the palette and F1 use, so the three
 *             can never disagree (principle P7, spec section 9.1).
 *   Appearance's `Color theme` row writes `<html data-theme>` through `lib/theme.ts` and records it
 *             in the layout store, which is the one place a setting has a side effect outside React.
 *   About     fills `This host` from the event log's `HostStatus` (`macOS 15.1 · arm64`), which is
 *             the only honest source for it.
 *
 * Row state is local: these are preferences with no persistence story yet (spec section 3.3 says
 * UI prefs go to localStorage, and the theme is the one that does).
 */
const TAB_ICON: Record<string, LucideIcon> = {
  sliders: SlidersHorizontal,
  palette: PaletteIcon,
  keyboard: Keyboard,
  shield: Shield,
  bell: Bell,
  databaseBackup: DatabaseBackup,
  info: Info,
};

type RowValue = boolean | string;

/** The tab bodies' shape, as `strings.settings` declares it. */
interface SettingsRow {
  kind: string;
  id: string;
  label: string;
  help?: string;
  value?: RowValue;
  options?: readonly string[];
  button?: string;
}

interface SettingsGroup {
  title?: string | null;
  rows: readonly SettingsRow[];
}

export function Settings() {
  const open = useOverlayStore((state) => state.settingsOpen);
  const tab = useOverlayStore((state) => state.settingsTab);
  const close = useOverlayStore((state) => state.closeSettings);
  const setTab = useOverlayStore((state) => state.openSettings);
  const openHub = useOverlayStore((state) => state.openHub);
  const hosts = useAppStore((state) => state.hosts);
  const theme = useLayoutStore((state) => state.theme);
  const setLayoutTheme = useLayoutStore((state) => state.setTheme);

  const [values, setValues] = useState<Record<string, RowValue>>(() =>
    collectDefaults(),
  );

  const host = localHost(hosts);

  const flip = (id: string): void => {
    setValues((current) => ({ ...current, [id]: current[id] !== true }));
  };

  const choose = (id: string, value: string): void => {
    setValues((current) => ({ ...current, [id]: value }));

    if (id === 'theme') {
      const next = value === 'Light' ? 'light' : 'dark';

      setTheme(next);
      setLayoutTheme(next);
    }
  };

  return (
    <Modal
      open={open}
      label={strings.topbar.settings.title}
      onClose={close}
      center
      className="settings-dlg flex max-h-[92vh] w-[min(820px,96vw)] overflow-hidden"
    >
      <nav
        className="flex w-[200px] shrink-0 flex-col gap-[2px] border-r border-border-subtle bg-bg-raised p-[10px] max-700:w-[64px]"
        aria-label={strings.topbar.settings.title}
      >
        {strings.settings.nav.map((item) => {
          const Icon = TAB_ICON[item.icon] ?? SlidersHorizontal;
          const selected = item.id === tab;

          return (
            <button
              key={item.id}
              type="button"
              data-settings-tab={item.id}
              aria-current={selected}
              className={
                'flex items-center gap-[10px] rounded-md px-[12px] py-[8px] text-left text-[12.5px] transition-colors duration-fast ease-ease ' +
                (selected
                  ? 'active bg-accent-subtle text-accent'
                  : 'text-text-secondary hover:bg-bg-hover hover:text-text-primary')
              }
              onClick={() => setTab(item.id as SettingsTab)}
            >
              <Icon size={14} aria-hidden="true" />
              <span className="max-700:hidden">{item.label}</span>
            </button>
          );
        })}
      </nav>

      <div className="min-w-0 flex-1 overflow-y-auto px-[28px] py-[22px]">
        {tab === 'keymap' ? <KeymapTab /> : null}

        {tab === 'about' ? (
          <AboutTab
            host={host === null ? '' : `${host.name} · ${host.platform}`}
            daemon={host?.sdcd ?? ''}
            onDoctor={() => {
              close();
              openHub('doctor');
            }}
          />
        ) : null}

        {tab === 'backup' ? <BackupTab /> : null}

        {tab !== 'keymap' && tab !== 'about' && tab !== 'backup' ? (
          <GroupTable
            table={tableFor(tab)}
            values={values}
            onFlip={flip}
            onChoose={choose}
            theme={theme}
          />
        ) : null}
      </div>
    </Modal>
  );
}

/** The rows table for a tab; `keymap`, `about` and `backup` have their own renderers. */
function tableFor(tab: SettingsTab): { title: string; desc: string; groups: readonly SettingsGroup[] } {
  switch (tab) {
    case 'appearance':
      return strings.settings.appearance;
    case 'safety':
      return strings.settings.safety;
    case 'notifications':
      return strings.settings.notifications;
    default:
      return strings.settings.general;
  }
}

/** Every toggle and select's starting value, read once from the strings table. */
function collectDefaults(): Record<string, RowValue> {
  const defaults: Record<string, RowValue> = {};

  for (const table of [
    strings.settings.general,
    strings.settings.appearance,
    strings.settings.safety,
    strings.settings.notifications,
  ]) {
    for (const group of table.groups as readonly SettingsGroup[]) {
      for (const row of group.rows) {
        if (row.value !== undefined) {
          defaults[row.id] = row.value;
        }
      }
    }
  }

  defaults['auto-backup'] = strings.settings.backup.autoBackup.value;

  return defaults;
}

/* ------------------------------------------------------------------------------------------------
 * The renderers
 * ---------------------------------------------------------------------------------------------- */

interface GroupTableProps {
  table: { title: string; desc: string; groups: readonly SettingsGroup[] };
  values: Record<string, RowValue>;
  onFlip: (id: string) => void;
  onChoose: (id: string, value: string) => void;
  theme: string;
}

/** One table of groups: the shape four of the seven tabs share. */
function GroupTable({ table, values, onFlip, onChoose, theme }: GroupTableProps) {
  return (
    <>
      <h2 className="text-[19px] font-semibold text-text-primary">{table.title}</h2>
      <p className="mb-[18px] text-[12.5px] text-text-muted">{table.desc}</p>

      {table.groups.map((group, index) => (
        <div key={group.title ?? index} className="mb-[22px]">
          {group.title === null || group.title === undefined ? null : (
            <h3 className="mb-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
              {group.title}
            </h3>
          )}

          {group.rows.map((row) => (
            <div
              key={row.id}
              data-setting-row={row.id}
              className="flex items-center gap-[12px] border-b border-border-subtle py-[11px] last:border-b-0"
            >
              <div className="min-w-0 flex-1">
                <div className="text-[13px] text-text-primary">{row.label}</div>
                {row.help === undefined ? null : (
                  <div className="text-[11.5px] text-text-muted">{row.help}</div>
                )}
              </div>

              <RowControl row={row} value={values[row.id]} onFlip={onFlip} onChoose={onChoose} theme={theme} />
            </div>
          ))}
        </div>
      ))}
    </>
  );
}

interface RowControlProps {
  row: SettingsRow;
  value: RowValue | undefined;
  onFlip: (id: string) => void;
  onChoose: (id: string, value: string) => void;
  theme: string;
}

/** The right-hand control: a switch, a select, or the sound-test button. */
function RowControl({ row, value, onFlip, onChoose, theme }: RowControlProps) {
  if (row.kind === 'sound-test') {
    return (
      <button
        type="button"
        className={BTN + ' ' + BTN_SECONDARY}
        onClick={() => toast('🔔 played')}
      >
        {row.button ?? 'Play'}
      </button>
    );
  }

  if (row.kind === 'select' || row.kind === 'theme') {
    const current = row.kind === 'theme' ? (theme === 'light' ? 'Light' : 'Dark') : String(value ?? '');

    return (
      <select
        className="select-mini rounded-md border border-border-default bg-bg-raised px-[10px] py-[5px] text-[12px] text-text-primary"
        aria-label={row.label}
        value={current}
        onChange={(event) => onChoose(row.id, event.target.value)}
      >
        {(row.options ?? []).map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    );
  }

  const on = value === true;

  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={row.label}
      className={'toggle' + (on ? ' on' : '')}
      onClick={() => onFlip(row.id)}
    />
  );
}

/** Keymap (spec section 9.11): the registry, grouped - Global (10), Session (6), Editor. */
function KeymapTab() {
  const sections = commandsByGroup();

  return (
    <>
      <h2 className="text-[19px] font-semibold text-text-primary">{strings.settings.nav[2].label}</h2>
      <p className="mb-[18px] text-[12.5px] text-text-muted">
        All shortcuts — one registry, so this list cannot drift from the keyboard.
      </p>

      {sections.map((section) => (
        <div key={section.group} className="mb-[18px]">
          <h3 className="mb-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
            {section.label} ({section.rows.length})
          </h3>

          {section.rows.map((command) => (
            <div
              key={command.id}
              data-keymap-row={command.id}
              className="flex items-center gap-[12px] border-b border-border-subtle py-[9px] text-[12.5px] last:border-b-0"
            >
              <span className="flex-1 text-text-primary">{command.label}</span>
              {command.inInput === true ? (
                <span className="text-[10px] text-text-muted" title={strings.keymap.firesInInputs}>
                  ⌨
                </span>
              ) : null}
              <span className="kbd inline-flex items-center rounded-[3px] border border-border-default border-b-2 bg-bg-base px-[6px] py-[1px] font-mono text-[10.5px] text-text-secondary">
                {command.hint ?? ''}
              </span>
            </div>
          ))}
        </div>
      ))}

      {/* The Editor group is the one row here that is not a shortcut (spec section 9.11). */}
      <div className="mb-[18px]">
        <h3 className="mb-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
          {strings.settings.keymapEditor.title}
        </h3>
        {strings.settings.keymapEditor.rows.map((row) => (
          <div key={row.id} data-setting-row={row.id} className="flex items-center gap-[12px] py-[11px]">
            <span className="flex-1 text-[13px] text-text-primary">{row.label}</span>
            <select
              className="select-mini rounded-md border border-border-default bg-bg-raised px-[10px] py-[5px] text-[12px] text-text-primary"
              aria-label={row.label}
              defaultValue={row.value}
            >
              {row.options.map((option) => (
                <option key={option} value={option}>
                  {option}
                </option>
              ))}
            </select>
          </div>
        ))}
      </div>
    </>
  );
}

/**
 * About (spec section 9.11): versions, this host, the three diagnostics and telemetry.
 *
 * The three version rows are filled from the things they describe - `package.json` for this window,
 * the event log's `HostStatus` for the daemon, `protocol/types.ts` for the protocol - because as
 * literals they were wrong: on a 0.7.5 build the dialog still said `v0.4.4`. `StatusBar` reads the same
 * two halves for its version cell, so the two cannot drift either.
 */
function AboutTab({
  host,
  daemon,
  onDoctor,
}: {
  host: string;
  daemon: string;
  onDoctor: () => void;
}) {
  const about = strings.settings.about;
  const versions: Record<string, string> = {
    app: `v${APP_VERSION}`,
    daemon: daemon === '' ? 'not reported' : `v${daemon}`,
    protocol: SDCP_VERSION,
  };

  return (
    <>
      <h2 className="text-[19px] font-semibold text-text-primary">{about.title}</h2>
      <p className="mb-[18px] text-[12.5px] text-text-muted">{about.desc}</p>

      <div className="mb-[22px]">
        {about.rows.map((row) => (
          <Row key={row.id} label={row.label}>
            <span className="font-mono text-[12px] text-text-secondary">{versions[row.id] ?? ''}</span>
          </Row>
        ))}

        <Row label={about.hostRow}>
          <span className="font-mono text-[12px] text-text-secondary">{host}</span>
        </Row>
      </div>

      <div className="mb-[22px]">
        <h3 className="mb-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
          {about.diagnostics}
        </h3>
        <div className="flex flex-wrap gap-[8px]">
          <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={onDoctor}>
            <Stethoscope size={12} aria-hidden="true" />
            {about.runDoctor}
          </button>
          <button
            type="button"
            className={BTN + ' ' + BTN_SECONDARY}
            onClick={() => toast(about.bundleToast)}
          >
            {about.createBundle}
          </button>
          <button
            type="button"
            className={BTN + ' ' + BTN_SECONDARY}
            onClick={() => toast(about.updatesToast)}
          >
            {about.checkUpdates}
          </button>
        </div>
      </div>

      <div className="mb-[22px]">
        <h3 className="mb-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
          {about.telemetry}
        </h3>
        <Row label={about.telemetryLabel} help={about.telemetryHelp}>
          <span className="toggle" aria-hidden="true" />
        </Row>
      </div>
    </>
  );
}

/** Backup (spec section 9.11): the auto-backup switch, the two archives and the two buttons. */
function BackupTab() {
  const backup = strings.settings.backup;

  return (
    <>
      <h2 className="text-[19px] font-semibold text-text-primary">{backup.title}</h2>
      <p className="mb-[18px] text-[12.5px] text-text-muted">{backup.desc}</p>

      <div className="mb-[22px]">
        <Row label={backup.autoBackup.label} help={backup.autoBackup.help}>
          <span className="toggle on" aria-hidden="true" />
        </Row>
      </div>

      <div className="mb-[22px]">
        <h3 className="mb-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
          {backup.recentTitle}
        </h3>

        {backup.archives.map((archive) => (
          <div
            key={archive.name}
            className="mb-[6px] flex items-center gap-[12px] rounded-md border border-border-subtle bg-bg-raised px-[14px] py-[12px]"
          >
            <DatabaseBackup
              size={16}
              aria-hidden="true"
              className={archive.fresh ? 'text-accent' : 'text-text-muted'}
            />
            <div className="min-w-0 flex-1">
              <div className="font-mono text-[12.5px] text-text-primary">{archive.name}</div>
              <div className="mt-[2px] text-[11px] text-text-muted">{archive.meta}</div>
            </div>
            <button
              type="button"
              className={BTN + ' ' + BTN_SECONDARY}
              onClick={() => toast(backup.restoreToast)}
            >
              {backup.restore}
            </button>
          </div>
        ))}

        <div className="mt-[14px] flex gap-[8px]">
          <button
            type="button"
            className={BTN + ' ' + BTN_PRIMARY}
            onClick={() => toast(backup.createToast)}
          >
            {backup.create}
          </button>
          <button
            type="button"
            className={BTN + ' ' + BTN_SECONDARY}
            onClick={() => toast(backup.restoreFromFileToast)}
          >
            {backup.restoreFromFile}
          </button>
        </div>
      </div>
    </>
  );
}

/** One label/control row - the shape the About and Backup tabs reuse. */
function Row({ label, help, children }: { label: string; help?: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-[12px] border-b border-border-subtle py-[11px] last:border-b-0">
      <div className="min-w-0 flex-1">
        <div className="text-[13px] text-text-primary">{label}</div>
        {help === undefined ? null : <div className="text-[11.5px] text-text-muted">{help}</div>}
      </div>
      {children}
    </div>
  );
}
