import { Gauge } from 'lucide-react';

import type { LocalTuning } from '../../../protocol/types';
import { strings } from '../strings';

/**
 * The local-model tuning proposal (P0-16), in the Provider Hub's Local flow: what this computer's GPU can carry for each
 * installed model, and the two Ollama settings that would let it. **A proposal only** - the card says so, and there is no
 * button that applies anything: `OLLAMA_FLASH_ATTENTION` and `OLLAMA_KV_CACHE_TYPE` are set on the Ollama *server* and
 * need it restarted, which is the person's decision.
 */
const t = strings.hub.tuning;

export function TuningCard({ tuning }: { tuning: LocalTuning }) {
  const gb = (bytes: number): string => (bytes / 1e9).toFixed(1);

  return (
    <div className="tuning-card mt-[12px] rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[10px] text-[11.5px] text-text-secondary" data-tuning="">
      <div className="mb-[4px] flex items-center gap-[6px] font-semibold text-text-primary">
        <Gauge size={13} aria-hidden="true" />
        {t.title}
      </div>

      <p className="m-0 text-text-muted">{tuning.hardware.gpu === null || tuning.hardware.gpuBytes === null ? t.noGpu : t.gpu(tuning.hardware.gpu, gb(tuning.hardware.gpuBytes))}</p>

      {tuning.proposals.map((proposal) => (
        <div key={proposal.model} className="mt-[8px]" data-tuning-model={proposal.model}>
          <p className="m-0">{proposal.fitsOnGpu ? t.fits(proposal.model, proposal.numCtx) : proposal.mode === 'gpu+cpu-experts' ? t.fits(proposal.model, proposal.numCtx) : t.spills(proposal.model)}</p>
          {proposal.mode === 'gpu+cpu-experts' && proposal.offload ? <p className="m-0 mt-[2px] text-text-muted">{t.moeExperts(gb(proposal.offload.gpuBytes), gb(proposal.offload.cpuBytes))}</p> : null}
          {proposal.mode === 'partial-offload' && proposal.offload ? <p className="m-0 mt-[2px] text-text-muted">{t.partial(gb(proposal.offload.cpuBytes), gb(proposal.offload.gpuBytes))}</p> : null}
          {proposal.estimate ? <p className="m-0 mt-[2px] text-state-waiting">{t.estimate}</p> : null}

          {proposal.env.length === 0 ? null : (
            <>
              <p className="mb-[2px] mt-[4px] text-text-muted">{t.env}</p>
              <pre className="m-0 rounded-sm bg-bg-input px-[8px] py-[4px] font-mono text-[11px] text-text-primary">{proposal.env.map((variable) => `${variable.name}=${variable.value}`).join('\n')}</pre>
              {proposal.needsRestart ? <p className="mt-[2px] text-text-muted">{t.restart}</p> : null}
            </>
          )}
        </div>
      ))}

      <p className="mb-0 mt-[8px] text-[10.5px] text-text-muted">{t.proposalOnly}</p>
    </div>
  );
}
