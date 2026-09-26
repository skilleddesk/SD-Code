import { Check, ChevronDown, ChevronRight, Plug, ShieldCheck } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';

import { strings } from '../../strings';
import { refreshCatalog } from '../../store/intents';
import {
  filterGroups,
  groupCatalog,
  tierLabel,
  TIERS,
  useModelStore,
  type CatalogGroup,
  type CatalogModel,
} from '../../store/model';
import { useOverlayStore } from '../../store/overlays';
import { useProviderStore } from '../../store/providers';
import { engineIcon, tierIcon } from './modelIcons';

/**
 * `.model-dropdown` - the Tier / Model picker of spec section 9.3, as v4 changed it.
 *
 * It opens *upward* (`bottom: calc(100% + 8px)`) because its trigger sits in the prompt toolbar at
 * the bottom of the window: 380px wide, up to 520px tall, on `--bg-overlay` with the XL shadow. Only
 * one can be open at a time, and that is store state rather than local state, so a click anywhere
 * outside can close it - the listener lives in `ModelSelector`, which owns the trigger.
 *
 *   TIER    Fast / Balanced / Deep, with what each one is for
 *   MODEL   one group per **connected** provider, with its connection badge (`CLI · signed in`,
 *           `API key · verified`), the newest two versions of each family, and the rest behind
 *           `Older versions (n)`
 *
 * What v4 removed, and why (docs/ROADMAP-v4.md, decision 6): a provider that is not connected no
 * longer has a group. Its models could not run, so offering them was the menu lying (P4). The menu
 * counts them in one line instead - `2 providers not connected · Manage in Provider Hub` - so the way
 * to connect them stays one click away. The footer's cost range went with it: `~$0.10 – $0.28` was a
 * constant, not an estimate of anything.
 *
 * The icon maps live in `./modelIcons` because the trigger shows the tier's icon too, and the two
 * must not disagree about what "Fast" looks like.
 */

/** Tone of the 22x22 chip: the selected row's is accented, everything else is neutral. */
const CHIP = 'mdd-icon grid h-[22px] w-[22px] shrink-0 place-items-center rounded-sm';
const CHIP_IDLE = CHIP + ' border border-border-subtle bg-bg-raised text-text-muted';
const CHIP_ON =
  CHIP + ' border border-accent-fill bg-accent-fill text-text-on-accent shadow-[0_0_10px_var(--accent-glow)]';

const ROW =
  'mdd-item flex cursor-pointer items-center gap-[10px] rounded-md px-[10px] py-[8px] text-[12.5px] transition-colors duration-fast ease-ease';
const ROW_IDLE = ROW + ' text-text-secondary hover:bg-bg-hover hover:text-text-primary';
const ROW_ON = ROW + ' selected bg-accent-subtle text-text-primary';

const GROUP_TITLE =
  'mdd-title flex items-center gap-[8px] px-[10px] pb-[6px] pt-[10px] text-[10px] font-bold uppercase tracking-[.1em] text-text-muted';
const BADGE = 'ml-auto font-mono text-[9.5px] font-medium normal-case tracking-normal';

const NAME = 'mdd-name text-[12.5px] font-medium text-text-primary';
const DESC = 'mdd-desc mt-[1px] font-mono text-[10.5px] text-text-muted';

const LINK =
  'flex cursor-pointer items-center gap-[6px] rounded-md px-[10px] py-[6px] text-[11px] hover:bg-bg-hover';

/** A row's two-line body: the name, then the mono one-liner under it. */
function Row({ name, description }: { name: string; description: string }) {
  return (
    <div className="mdd-body min-w-0 flex-1">
      <div className={NAME}>{name}</div>
      <div className={DESC}>{description}</div>
    </div>
  );
}

/** A clickable div that answers Enter the way a button does. */
function pressable(action: () => void) {
  return {
    role: 'button' as const,
    tabIndex: 0,
    onClick: action,
    onKeyDown: (event: React.KeyboardEvent) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        action();
      }
    },
  };
}

/** How a group reaches its models - what the badge beside its name says. */
function viaOf(group: CatalogGroup): 'cli' | 'api' | 'local' {
  if (group.engine !== 'native_api') {
    return 'cli';
  }

  return group.providerId === 'ollama' ? 'local' : 'api';
}

export function ModelDropdown() {
  const { tier, providerId, model, catalog, setTier, choose } = useModelStore();
  const providers = useProviderStore((state) => state.providers);
  const openHub = useOverlayStore((state) => state.openHub);
  /* Which groups have their older versions open. Local: it is a view choice, not a fact. */
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  /** The search line (0.9.0). Local: a filter is a view choice, and it resets when the menu closes. */
  const [query, setQuery] = useState('');

  /*
   * Opening the dropdown re-reads the catalogue.
   *
   * It is a local read of a list the daemon already holds (no provider is contacted unless the Hub's
   * `Refresh` asks for it), so it costs nothing and it means the menu can never be a launch behind: sign
   * a CLI in, open this, and its models are here.
   */
  useEffect(() => {
    void refreshCatalog();
  }, []);

  const { groups, disconnected } = useMemo(() => groupCatalog(catalog, providers), [catalog, providers]);
  const searching = query.trim() !== '';
  const shown = useMemo(() => filterGroups(groups, query), [groups, query]);

  const manage = (): void => {
    useModelStore.getState().closeDropdown();
    openHub();
  };

  const toggleOlder = (id: string): void => {
    setExpanded((current) => {
      const next = new Set(current);

      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }

      return next;
    });
  };

  const renderModel = (group: CatalogGroup, row: CatalogModel) => {
    const EngineIcon = engineIcon(group.engine);
    const selected = row.id === model && row.providerId === providerId;
    const pick = (): void =>
      choose({ engine: group.engine, providerId: group.providerId, model: row.id, tier: row.tier });

    return (
      <div
        key={`${group.providerId}:${row.id}`}
        className={selected ? ROW_ON : ROW_IDLE}
        aria-selected={selected}
        data-model-row={row.id}
        {...pressable(pick)}
      >
        <div className={selected ? CHIP_ON : CHIP_IDLE}>
          <EngineIcon size={12} aria-hidden="true" />
        </div>
        <Row
          name={row.name === '' ? row.id : row.name}
          description={`${row.id} · ${tierLabel(row.tier)}${row.cost === '' ? ` · ${row.source}` : ` · ${row.cost}`}`}
        />
        {selected ? <Check size={14} className="shrink-0 text-accent" aria-hidden="true" /> : null}
      </div>
    );
  };

  return (
    <div
      className="model-dropdown absolute bottom-[calc(100%+8px)] left-0 z-[300] max-h-[520px] w-[380px] max-w-[calc(100vw-24px)] overflow-y-auto rounded-lg border border-border-default bg-bg-overlay p-[6px] shadow-xl animate-drop-up"
      role="dialog"
      aria-label={strings.prompt.model.groupTitles.model}
    >
      {/* The search line (0.9.0): filters by model name, id or provider. While it is in use the tier
          block steps aside - the list is the answer to what was typed. */}
      <div className="mdd-search px-[4px] pb-[2px] pt-[4px]">
        <input
          type="text"
          className="w-full rounded-md border border-border-subtle bg-bg-input px-[10px] py-[6px] text-[12px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
          placeholder={strings.prompt.model.search}
          value={query}
          data-model-search
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>

      {searching ? null : (
      <div className="mdd-group mb-[4px]">
        <div className={GROUP_TITLE}>{strings.prompt.model.groupTitles.tier}</div>
        {TIERS.map((candidate) => {
          const Icon = tierIcon(candidate.id);
          const selected = candidate.id === tier;

          return (
            <div
              key={candidate.id}
              className={selected ? ROW_ON : ROW_IDLE}
              aria-selected={selected}
              {...pressable(() => setTier(candidate.id))}
            >
              <div className={selected ? CHIP_ON : CHIP_IDLE}>
                <Icon size={12} aria-hidden="true" />
              </div>
              <Row name={candidate.label} description={candidate.description} />
              {selected ? <Check size={14} className="shrink-0 text-accent" aria-hidden="true" /> : null}
            </div>
          );
        })}
      </div>
      )}

      <div className="mdd-divider mx-[4px] my-[6px] h-px bg-border-subtle" />

      <div className={GROUP_TITLE}>
        {strings.prompt.model.groupTitles.model}
        <span className={BADGE}>
          {catalog.length === 0
            ? strings.prompt.model.catalogEmpty
            : strings.prompt.model.connectedCount(groups.length)}
        </span>
      </div>

      {searching && shown.length === 0 ? (
        <div className="mdd-none px-[10px] py-[8px] text-[12px] text-text-secondary" data-no-matches>
          {strings.prompt.model.noMatches(query.trim())}
        </div>
      ) : null}

      {groups.length === 0 && catalog.length > 0 ? (
        <div className="mdd-none px-[10px] py-[8px] text-[12px] leading-[1.5] text-text-secondary" data-none-connected>
          {strings.prompt.model.noneConnected}
        </div>
      ) : null}

      {shown.map((group) => {
        const open = expanded.has(group.providerId);

        return (
          <div key={group.providerId} className="mdd-group mb-[6px]" data-provider-group={group.providerId}>
            <div className={GROUP_TITLE}>
              <span className="text-text-secondary">{group.providerLabel}</span>
              <span className={BADGE + ' text-state-success'}>✓ {strings.prompt.model.via(viaOf(group))}</span>
            </div>

            {group.models.map((row) => renderModel(group, row))}

            {group.older.length === 0 ? null : (
              <>
                <div
                  className={LINK + ' text-text-muted'}
                  aria-expanded={open}
                  data-older-toggle={group.providerId}
                  {...pressable(() => toggleOlder(group.providerId))}
                >
                  {open ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}
                  {open ? strings.prompt.model.hideOlder : strings.prompt.model.older(group.older.length)}
                </div>
                {open ? group.older.map((row) => renderModel(group, row)) : null}
              </>
            )}
          </div>
        );
      })}

      <div
        className={LINK + ' mdd-action mt-[4px] border-t border-border-subtle py-[8px] text-accent hover:bg-accent-subtle'}
        data-manage-providers
        {...pressable(manage)}
      >
        <Plug size={12} aria-hidden="true" />
        {disconnected > 0 ? (
          <span>
            <span className="text-text-muted">{strings.prompt.model.disconnected(disconnected)} · </span>
            {strings.prompt.model.manage}
          </span>
        ) : (
          strings.prompt.model.connectMore
        )}
      </div>

      <div className="mdd-foot mt-[4px] flex items-center gap-[8px] border-t border-border-subtle px-[10px] py-[8px] font-mono text-[10.5px] text-text-muted">
        <ShieldCheck size={11} aria-hidden="true" />
        <span>{strings.prompt.model.footerNote}</span>
      </div>
    </div>
  );
}
