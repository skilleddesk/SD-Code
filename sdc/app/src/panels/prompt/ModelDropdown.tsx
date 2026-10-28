import { Check, Plug, Search } from 'lucide-react';
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

const ROW =
  'mdd-item group flex min-h-[34px] cursor-pointer items-center gap-[8px] rounded-md px-[10px] py-[6px] text-[12.5px] transition-colors duration-fast ease-ease';
const ROW_IDLE = ROW + ' text-text-secondary hover:bg-bg-hover hover:text-text-primary';
const ROW_ON = ROW + ' selected bg-accent-subtle text-text-primary';

const CHIP =
  'mdd-version shrink-0 rounded-full border px-[7px] font-mono text-[10.5px] leading-[18px] transition-colors duration-fast ease-ease';
const CHIP_IDLE = CHIP + ' border-border-subtle text-text-muted hover:border-border-strong hover:text-text-primary';
const CHIP_ON = CHIP + ' border-accent-fill bg-accent-fill text-text-on-accent';

const TAB =
  'mdd-tab shrink-0 cursor-pointer whitespace-nowrap rounded-full px-[10px] py-[3px] text-[11.5px] transition-colors duration-fast ease-ease';
const TAB_IDLE = TAB + ' text-text-muted hover:bg-bg-hover hover:text-text-primary';
const TAB_ON = TAB + ' bg-bg-raised font-medium text-text-primary shadow-[inset_0_0_0_1px_var(--border-subtle)]';

const PROVIDER_TITLE = 'mdd-title px-[10px] pb-[2px] pt-[8px] text-[10px] font-semibold uppercase tracking-[.08em] text-text-muted';

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

    return (
      <div
        key={family.key}
        className={selected ? ROW_ON : ROW_IDLE}
        aria-selected={selected}
        data-model-row={newest.row.id}
        title={newest.row.id}
        {...pressable(() => pick(group, newest.row))}
      >
        <span className="mdd-name min-w-0 flex-1 truncate font-medium text-text-primary">{family.label}</span>
        {chips.map(({ label, row }) => (
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
        {selected && chips.length === 0 ? <Check size={14} className="shrink-0 text-accent" aria-hidden="true" /> : null}
      </div>
    );
  };

  return (
    <div
      className="model-dropdown absolute bottom-[calc(100%+8px)] left-0 z-[300] flex max-h-[min(380px,70vh)] w-[320px] max-w-[calc(100vw-24px)] flex-col rounded-lg border border-border-default bg-bg-overlay p-[6px] shadow-xl animate-drop-up"
      role="dialog"
      aria-label={strings.prompt.model.groupTitles.model}
    >
      <label className="mdd-search flex items-center gap-[6px] rounded-md border border-border-subtle bg-bg-input px-[8px] focus-within:border-border-strong">
        <Search size={12} className="shrink-0 text-text-muted" aria-hidden="true" />
        <input
          type="text"
          className="min-w-0 flex-1 bg-transparent py-[6px] text-[12px] text-text-primary outline-none placeholder:text-text-muted focus-visible:outline-none"
          placeholder={strings.prompt.model.search}
          aria-label={strings.prompt.model.search}
          value={query}
          data-model-search
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>

      {!searching && groups.length > 1 ? (
        <div className="mdd-tabs mt-[6px] flex flex-wrap gap-[2px]" role="tablist">
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
              {group.providerLabel}
            </span>
          ))}
        </div>
      ) : null}

      <div className="mdd-list mt-[4px] min-h-0 flex-1 overflow-y-auto">
        {catalog.length === 0 ? (
          <div className="mdd-none px-[10px] py-[8px] text-[12px] text-text-secondary">{strings.prompt.model.catalogEmpty}</div>
        ) : groups.length === 0 ? (
          <div className="mdd-none px-[10px] py-[8px] text-[12px] leading-[1.5] text-text-secondary" data-none-connected>
            {strings.prompt.model.noneConnected}
          </div>
        ) : searching ? (
          shown.length === 0 ? (
            <div className="mdd-none px-[10px] py-[8px] text-[12px] text-text-secondary" data-no-matches>
              {strings.prompt.model.noMatches(query.trim())}
            </div>
          ) : (
            shown.map((group) => (
              <div key={group.providerId} className="mdd-group" data-provider-group={group.providerId}>
                {shown.length > 1 ? <div className={PROVIDER_TITLE}>{group.providerLabel}</div> : null}
                {familiesOf(rowsOf(group)).map((family) => renderFamily(group, family))}
              </div>
            ))
          )
        ) : active === undefined ? null : (
          <div className="mdd-group" data-provider-group={active.providerId}>
            {familiesOf(rowsOf(active)).map((family) => renderFamily(active, family))}
          </div>
        )}
      </div>

      <div
        className="mdd-action mt-[4px] flex cursor-pointer items-center gap-[6px] rounded-md border-t border-border-subtle px-[10px] py-[7px] text-[11.5px] text-text-muted hover:bg-bg-hover hover:text-text-primary"
        data-manage-providers
        {...pressable(manage)}
      >
        <Plug size={12} aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate">{strings.prompt.model.manageProviders}</span>
        {disconnected > 0 ? (
          <span className="shrink-0 rounded-full border border-border-subtle px-[6px] font-mono text-[10px] leading-[16px]" title={strings.prompt.model.disconnected(disconnected)}>
            +{disconnected}
          </span>
        ) : null}
      </div>
    </div>
  );
}
