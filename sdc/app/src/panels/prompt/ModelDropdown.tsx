import {
  Check,
  Plug,
  Plus,
  ShieldCheck,
} from 'lucide-react';
import { useEffect, useMemo } from 'react';

import { strings } from '../../strings';
import { refreshCatalog } from '../../store/intents';
import {
  connectModeFor,
  groupCatalog,
  tierLabel,
  TIERS,
  useModelStore,
} from '../../store/model';
import { useOverlayStore } from '../../store/overlays';
import { useProviderStore } from '../../store/providers';
import { engineIcon, tierIcon } from './modelIcons';

/**
 * `.model-dropdown` - the Tier / Engine / Model picker of spec section 9.3.
 *
 * It opens *upward* (`bottom: calc(100% + 8px)`) because its trigger sits in the prompt toolbar at
 * the bottom of the window: 380px wide, up to 520px tall, on `--bg-overlay` with the XL shadow. Only
 * one can be open at a time, and that is store state rather than local state, so a click anywhere
 * outside can close it - the listener lives in `ModelSelector`, which owns the trigger.
 *
 * Three groups, separated by hairlines:
 *
 *   TIER    Fast / Balanced / Deep, with what each one is for
 *   ENGINE  the four ways to reach a model - three CLIs and the native API
 *   MODEL   whatever the selected engine offers, which is why the list changes with the engine
 *
 * Every row is the same object: a 22x22 icon chip that fills in accented when the row is selected, a
 * two-line body, and a check. Under them sit the escape hatch (connect something else, which opens
 * the Provider Hub) and the promise (`Every change visible` plus the cost estimate).
 *
 * The icon maps live in `./modelIcons` because the trigger shows the tier's icon too, and the two
 * must not disagree about what "Fast" looks like - and because a file that exports a component next
 * to a helper loses hot reloading.
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
  'mdd-title px-[10px] pb-[6px] pt-[10px] text-[10px] font-bold uppercase tracking-[.1em] text-text-muted';

const NAME = 'mdd-name text-[12.5px] font-medium text-text-primary';
const DESC = 'mdd-desc mt-[1px] font-mono text-[10.5px] text-text-muted';

/** A row's two-line body: the name, then the mono one-liner under it. */

/** A row's two-line body: the name, then the mono one-liner under it. */
function Row({ name, description }: { name: string; description: string }) {
  return (
    <div className="mdd-body min-w-0 flex-1">
      <div className={NAME}>{name}</div>
      <div className={DESC}>{description}</div>
    </div>
  );
}

export function ModelDropdown() {
  const { tier, providerId, model, catalog, setTier, choose } = useModelStore();
  const providers = useProviderStore((state) => state.providers);
  const openConnect = useOverlayStore((state) => state.openConnect);
  const openHub = useOverlayStore((state) => state.openHub);

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

  const groups = useMemo(() => groupCatalog(catalog, providers), [catalog, providers]);
  const connected = groups.filter((group) => group.connected).length;

  const connectMore = (): void => {
    useModelStore.getState().closeDropdown();
    openHub();
  };

  return (
    <div
      className="model-dropdown absolute bottom-[calc(100%+8px)] left-0 z-[300] max-h-[520px] w-[380px] overflow-y-auto rounded-lg border border-border-default bg-bg-overlay p-[6px] shadow-xl animate-drop-up"
      role="dialog"
      aria-label={strings.prompt.model.groupTitles.model}
    >
      <div className="mdd-group mb-[4px]">
        <div className={GROUP_TITLE}>{strings.prompt.model.groupTitles.tier}</div>
        {TIERS.map((candidate) => {
          const Icon = tierIcon(candidate.id);
          const selected = candidate.id === tier;

          return (
            <div
              key={candidate.id}
              className={selected ? ROW_ON : ROW_IDLE}
              role="button"
              tabIndex={0}
              aria-selected={selected}
              onClick={() => setTier(candidate.id)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  setTier(candidate.id);
                }
              }}
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

      <div className="mdd-divider mx-[4px] my-[6px] h-px bg-border-subtle" />

      <div className={GROUP_TITLE}>
        {strings.prompt.model.groupTitles.model}
        <span className="mdd-verified font-mono text-[9.5px] normal-case tracking-normal">
          {groups.length === 0
            ? strings.prompt.model.catalogEmpty
            : strings.prompt.model.verifiedCount(connected, groups.length)}
        </span>
      </div>

      {groups.map((group) => {
        const EngineIcon = engineIcon(group.engine);
        const mode = connectModeFor(group.providerId);

        return (
          <div key={group.providerId} className="mdd-group mb-[6px]">
            <div className={GROUP_TITLE}>
              <span className={group.connected ? 'text-state-success' : 'text-state-waiting'}>
                {group.providerLabel}
              </span>
              <span className="font-mono text-[9.5px] normal-case tracking-normal">
                {group.connected ? strings.prompt.model.verified : strings.prompt.model.notConnected}
              </span>
            </div>

            {group.connected ? null : (
              <div
                className="mdd-connect flex cursor-pointer items-center gap-[8px] rounded-md px-[10px] py-[7px] text-[12px] text-accent hover:bg-accent-subtle"
                role="button"
                tabIndex={0}
                data-connect-provider={group.providerId}
                onClick={() => openConnect(group.providerId, mode)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    openConnect(group.providerId, mode);
                  }
                }}
              >
                <Plug size={12} aria-hidden="true" />
                {strings.prompt.model.connect(mode, group.providerLabel)}
              </div>
            )}

            {group.models.map((row) => {
              const selected = row.id === model && row.providerId === providerId;

              return (
                <div
                  key={`${group.providerId}:${row.id}`}
                  className={selected ? ROW_ON : ROW_IDLE}
                  role="button"
                  tabIndex={0}
                  aria-selected={selected}
                  data-model-row={row.id}
                  onClick={() =>
                    choose({ engine: group.engine, providerId: group.providerId, model: row.id, tier: row.tier })
                  }
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      choose({ engine: group.engine, providerId: group.providerId, model: row.id, tier: row.tier });
                    }
                  }}
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
            })}
          </div>
        );
      })}

      <div
        className="mdd-action mt-[4px] flex cursor-pointer items-center gap-[6px] rounded-md border-t border-border-subtle px-[10px] py-[8px] text-[11px] text-accent hover:bg-accent-subtle"
        role="button"
        tabIndex={0}
        onClick={connectMore}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            connectMore();
          }
        }}
      >
        <Plus size={12} aria-hidden="true" />
        {strings.prompt.model.connectMore}
      </div>

      <div className="mdd-foot mt-[4px] flex items-center gap-[8px] border-t border-border-subtle px-[10px] py-[8px] font-mono text-[10.5px] text-text-muted">
        <ShieldCheck size={11} aria-hidden="true" />
        <span>{strings.prompt.model.footerNote}</span>
        <span className="mdd-cost ml-auto font-medium text-text-secondary">
          {strings.prompt.model.footerCost}
        </span>
      </div>
    </div>
  );
}
