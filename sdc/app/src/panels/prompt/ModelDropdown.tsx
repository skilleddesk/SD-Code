import { Check, ChevronRight, Plug, Search } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';

import { strings } from '../../strings';
import { refreshCatalog } from '../../store/intents';
import {
  familiesOf,
  filterGroups,
  groupCatalog,
  useModelStore,
  type CatalogGroup,
  type CatalogModel,
  type ModelFamily,
} from '../../store/model';
import { chooseForChat, useChatModel } from '../../store/chatModel';
import { useOverlayStore } from '../../store/overlays';
import { useProviderStore } from '../../store/providers';
import { tierIcon } from './modelIcons';

/**
 * `.model-dropdown` - the model picker above the prompt box (v5).
 *
 * It opens *upward* because its trigger sits in the prompt toolbar at the bottom of the window. The owner's
 * complaint about v4 was its length: every connected provider's models stacked one under the other, a tier block
 * and an effort row above them. v5 keeps only what a pick needs - the model's name and its version:
 *
 *   [ Search models…                    ]
 *   [Claude Code] [Codex] [Groq]            one tab per connected provider (only when there is more than one)
 *   ✦ Claude Opus          5.5   5   4.8     one line per model family; its versions are chips, newest first
 *   ✦ Claude Sonnet        5.5   5
 *   Manage providers                        one line, with the count of providers not connected
 *
 * So the height is one provider's families - a handful of lines - however many providers are connected. Pressing
 * a line picks the family's newest version; pressing a chip picks that version. Typing searches every provider.
 *
 * The list is never a launch behind: opening the menu re-reads the catalogue, and the daemon asks the providers
 * again in the background when its list is more than a few minutes old - a new version arrives in the open menu
 * through `ModelsUpdated`.
 */

/*
 * 1.0.3 look - the report: "the menu does not match SDC's design; make it modern". The same glass as every SDC
 * popover, a search field that is part of the sheet rather than a box inside it, provider tabs as one segmented
 * strip, and each family as a line with its tier badge, its name and context underneath, and its versions as a
 * compact segmented set on the right.
 */
const ROW =
  'mdd-item group relative flex min-h-[46px] cursor-pointer items-center gap-[10px] rounded-[10px] px-[8px] py-[6px] text-[12.5px] outline-none transition-colors duration-fast ease-ease focus-visible:bg-bg-hover';
const ROW_IDLE = ROW + ' text-text-secondary hover:bg-bg-hover hover:text-text-primary';
const ROW_ON = ROW + ' selected bg-accent-subtle text-text-primary shadow-[inset_0_0_0_1px_var(--accent-glow)]';

const BADGE = 'grid h-[30px] w-[30px] shrink-0 place-items-center rounded-[9px] border transition-colors duration-fast';
const BADGE_IDLE = BADGE + ' border-border-subtle bg-bg-raised text-text-muted group-hover:text-accent';
const BADGE_ON = BADGE + ' brand-fill border-transparent';

const CHIP =
  'mdd-version shrink-0 cursor-pointer rounded-full px-[8px] font-mono text-[10.5px] leading-[20px] outline-none transition-colors duration-fast ease-ease focus-visible:ring-1 focus-visible:ring-accent';
const CHIP_IDLE = CHIP + ' text-text-muted hover:bg-bg-active hover:text-text-primary';
const CHIP_ON = CHIP + ' bg-accent-fill font-semibold text-text-on-accent shadow-sm';

const TAB =
  'mdd-tab inline-flex shrink-0 cursor-pointer items-center gap-[6px] whitespace-nowrap rounded-full px-[10px] py-[4px] text-[11.5px] outline-none transition-colors duration-fast ease-ease focus-visible:ring-1 focus-visible:ring-accent';
const TAB_IDLE = TAB + ' text-text-muted hover:bg-bg-hover hover:text-text-primary';
const TAB_ON = TAB + ' bg-bg-active font-medium text-text-primary shadow-[inset_0_0_0_1px_var(--border-default)]';

const PROVIDER_TITLE = 'mdd-title px-[10px] pb-[4px] pt-[10px] text-[10px] font-semibold uppercase tracking-[.09em] text-text-muted';

/** `200000` -> `200K context`; nothing when the size is not known. */
function contextLine(ctx: number): string {
  if (!(ctx > 0)) {
    return '';
  }

  return ctx >= 1_000_000 ? `${Math.round(ctx / 100_000) / 10}M context` : `${Math.round(ctx / 1000)}K context`;
}

/** A clickable element that answers Enter and Space the way a button does. */
function pressable(action: () => void) {
  return {
    role: 'button' as const,
    tabIndex: 0,
    onClick: (event: React.MouseEvent) => {
      event.stopPropagation();
      action();
    },
    onKeyDown: (event: React.KeyboardEvent) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        event.stopPropagation();
        action();
      }
    },
  };
}

/** A group's rows, newest of each family first and its recent versions behind it. */
function rowsOf(group: CatalogGroup): CatalogModel[] {
  return [...group.models, ...group.older];
}

export function ModelDropdown({ sessionId }: { sessionId: string | null }) {
  const { catalog, inUse } = useModelStore();
  /* This box's chat, and only it (0.16.1): a pick here never changes another chat's model. */
  const { providerId, model } = useChatModel(sessionId);
  const providers = useProviderStore((state) => state.providers);
  const openHub = useOverlayStore((state) => state.openHub);
  /** The search line. Local: a filter is a view choice, and it resets when the menu closes. */
  const [query, setQuery] = useState('');

  useEffect(() => {
    void refreshCatalog();
  }, []);

  const { groups, disconnected } = useMemo(() => groupCatalog(catalog, providers, inUse), [catalog, providers, inUse]);
  const searching = query.trim() !== '';
  const shown = useMemo(() => filterGroups(groups, query), [groups, query]);

  /* The tab opens on the provider of the chat's model, else the first connected one. */
  const [tab, setTab] = useState<string | null>(null);
  const activeId = tab ?? (groups.some((group) => group.providerId === providerId) ? providerId : groups[0]?.providerId) ?? null;
  const active = groups.find((group) => group.providerId === activeId) ?? groups[0];

  const close = (): void => useModelStore.getState().closeDropdown();
  const pick = (group: CatalogGroup, row: CatalogModel): void => {
    chooseForChat(sessionId, { engine: group.engine, providerId: group.providerId, model: row.id, tier: row.tier });
    close();
  };
  const manage = (): void => {
    close();
    openHub();
  };

  const renderFamily = (group: CatalogGroup, family: ModelFamily) => {
    const newest = family.versions[0];

    if (newest === undefined) {
      return null;
    }

    const isOn = (row: CatalogModel): boolean => row.id === model && row.providerId === providerId;
    const selected = family.versions.some(({ row }) => isOn(row));
    const chips = family.versions.filter((version) => version.label !== '');
    const TierIcon = tierIcon(newest.row.tier);
    /* A name without a version (`deepseek-flash`) is the provider's own alias for whatever it serves newest - DeepSeek's
       API offers V4.1 Flash only as `deepseek-flash` (it refuses `deepseek-v4.1-flash`, measured 2026-10-28). Said so,
       rather than leaving the row looking older than it is. */
    const detail = chips.length === 0 ? strings.prompt.model.current(newest.row.id) : contextLine(newest.row.ctx);

    return (
      <div
        key={family.key}
        className={selected ? ROW_ON : ROW_IDLE}
        aria-selected={selected}
        data-model-row={newest.row.id}
        title={newest.row.id}
        {...pressable(() => pick(group, newest.row))}
      >
        <span className={selected ? BADGE_ON : BADGE_IDLE} aria-hidden="true">
          {selected && chips.length === 0 ? <Check size={14} strokeWidth={2.6} /> : <TierIcon size={14} />}
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="mdd-name truncate font-medium leading-[1.3] text-text-primary">{family.label}</span>
          <span className="truncate font-mono text-[10px] leading-[1.4] text-text-muted">{detail === '' ? newest.row.id : detail}</span>
        </span>
        {chips.length === 0 ? null : (
          <span className="flex shrink-0 items-center gap-[1px] rounded-full border border-border-subtle bg-bg-input p-[2px]">
            {chips.slice(0, 4).map(({ label, row }) => (
              <span
                key={row.id}
                className={isOn(row) ? CHIP_ON : CHIP_IDLE}
                aria-pressed={isOn(row)}
                aria-label={`${family.label} ${label}`}
                title={row.id}
                data-model-version={row.id}
                {...pressable(() => pick(group, row))}
              >
                {label}
              </span>
            ))}
          </span>
        )}
      </div>
    );
  };

  return (
    <div
      className="model-dropdown absolute bottom-[calc(100%+10px)] left-0 z-[300] flex max-h-[min(440px,72vh)] w-[360px] max-w-[calc(100vw-24px)] flex-col overflow-hidden rounded-[16px] border border-border-default bg-bg-overlay shadow-xl animate-drop-up"
      role="dialog"
      aria-label={strings.prompt.model.groupTitles.model}
    >
      <label className="mdd-search flex shrink-0 items-center gap-[8px] border-b border-border-subtle px-[14px]">
        <Search size={14} className="shrink-0 text-text-muted" aria-hidden="true" />
        <input
          type="text"
          className="min-w-0 flex-1 bg-transparent py-[12px] text-[12.5px] text-text-primary outline-none placeholder:text-text-muted focus-visible:outline-none"
          placeholder={strings.prompt.model.search}
          aria-label={strings.prompt.model.search}
          value={query}
          data-model-search
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>

      {!searching && groups.length > 1 ? (
        <div className="mdd-tabs flex shrink-0 gap-[4px] overflow-x-auto border-b border-border-subtle px-[10px] py-[8px] [scrollbar-width:none]" role="tablist">
          {groups.map((group) => (
            <span
              key={group.providerId}
              role="tab"
              tabIndex={0}
              aria-selected={group.providerId === active?.providerId}
              className={group.providerId === active?.providerId ? TAB_ON : TAB_IDLE}
              data-provider-tab={group.providerId}
              onClick={() => setTab(group.providerId)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  setTab(group.providerId);
                }
              }}
            >
              <span className={'h-[6px] w-[6px] rounded-full ' + (group.needsAuth === true ? 'bg-state-waiting' : 'bg-state-success')} aria-hidden="true" />
              {group.providerLabel}
            </span>
          ))}
        </div>
      ) : null}

      <div className="mdd-list min-h-0 flex-1 overflow-y-auto p-[6px]">
        {catalog.length === 0 ? (
          <div className="mdd-none px-[10px] py-[14px] text-center text-[12px] text-text-secondary">{strings.prompt.model.catalogEmpty}</div>
        ) : groups.length === 0 ? (
          <div className="mdd-none px-[10px] py-[14px] text-center text-[12px] leading-[1.5] text-text-secondary" data-none-connected>
            {strings.prompt.model.noneConnected}
          </div>
        ) : searching ? (
          shown.length === 0 ? (
            <div className="mdd-none px-[10px] py-[14px] text-center text-[12px] text-text-secondary" data-no-matches>
              {strings.prompt.model.noMatches(query.trim())}
            </div>
          ) : (
            shown.map((group) => (
              <div key={group.providerId} className="mdd-group flex flex-col gap-[2px]" data-provider-group={group.providerId}>
                {shown.length > 1 ? <div className={PROVIDER_TITLE}>{group.providerLabel}</div> : null}
                {familiesOf(rowsOf(group)).map((family) => renderFamily(group, family))}
              </div>
            ))
          )
        ) : active === undefined ? null : (
          <div className="mdd-group flex flex-col gap-[2px]" data-provider-group={active.providerId}>
            {familiesOf(rowsOf(active)).map((family) => renderFamily(active, family))}
          </div>
        )}
      </div>

      <div
        className="mdd-action group flex shrink-0 cursor-pointer items-center gap-[8px] border-t border-border-subtle bg-bg-raised px-[14px] py-[10px] text-[11.5px] text-text-muted outline-none transition-colors duration-fast hover:bg-bg-hover hover:text-text-primary focus-visible:bg-bg-hover"
        data-manage-providers
        {...pressable(manage)}
      >
        <Plug size={13} aria-hidden="true" className="text-accent" />
        <span className="min-w-0 flex-1 truncate">{strings.prompt.model.manageProviders}</span>
        {disconnected > 0 ? (
          <span className="shrink-0 rounded-full bg-bg-active px-[7px] font-mono text-[10px] leading-[18px]" title={strings.prompt.model.disconnected(disconnected)}>
            +{disconnected}
          </span>
        ) : null}
        <ChevronRight size={13} aria-hidden="true" className="shrink-0 transition-transform duration-fast group-hover:translate-x-[2px]" />
      </div>
    </div>
  );
}
