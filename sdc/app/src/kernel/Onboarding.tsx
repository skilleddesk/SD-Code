import { Check, FolderOpen, Globe, History, OctagonX, Plug, Server, Sparkles, Stethoscope } from 'lucide-react';

import { BrandLogo } from '../panels/ui/BrandLogo';
import { useEffect, useState } from 'react';

import { strings } from '../strings';
import { openFolder, runDoctor } from '../store/intents';
import { useKernelUi } from '../store/kernelUi';
import { useOverlayStore } from '../store/overlays';
import { useAppStore } from '../store/store';
import { Modal } from '../modals/Modal';
import { BTN, BTN_LG, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';

/**
 * **The first ten minutes** (0.12, the plan's onboarding wizard): a goal, a model, a health check of this
 * machine, the first real task, and the three things that make SDC safe to try - the Time Machine, the
 * Intent Contract and the kill switch - shown before they are needed. It opens once, on the first launch,
 * and from the palette any time after.
 */
const k = strings.kernel.onboarding;
const DONE = 'sdc.onboarded.v1';

type Goal = 'local' | 'vps' | 'new' | 'wordpress';

const GOALS: readonly { id: Goal; icon: typeof FolderOpen }[] = [
  { id: 'local', icon: FolderOpen },
  { id: 'vps', icon: Server },
  { id: 'new', icon: Sparkles },
  { id: 'wordpress', icon: Globe },
];

export function Onboarding() {
  const open = useKernelUi((state) => state.onboardingOpen);
  const close = useKernelUi((state) => state.closeOnboarding);
  const providers = useAppStore((state) => state.providers.filter((provider) => provider.status === 'connected').length);
  /* The map, not `?? []`: a selector that builds a new empty array loops React (error #185). */
  const doctorByHost = useAppStore((state) => state.doctor);
  const doctor = doctorByHost.local ?? [];
  /* A dialog this one opened (the Provider Hub, then Connect; Add a server; Settings) is drawn on top of it.
     Rendered last in App, the welcome dialog used to cover them instead: on a fresh install - a new Mac -
     "Open the Provider Hub" opened the hub underneath it, and Sign in looked like it did nothing (0.15.9,
     seen on a real Apple Silicon Mac). It steps aside while one is open and comes back on the same step. */
  const covered = useOverlayStore(
    (state) => state.hubOpen || state.connectOpen || state.addHostOpen || state.settingsOpen || state.newProjectOpen,
  );
  const [step, setStep] = useState(0);
  const [goal, setGoal] = useState<Goal>('local');

  useEffect(() => {
    if (open) {
      setStep(0);
    }
  }, [open]);

  const finish = (): void => {
    try {
      globalThis.localStorage?.setItem(DONE, 'yes');
    } catch {
      /* It opens again next time; nothing worse. */
    }

    close();
  };

  const firstTask = (): void => {
    finish();

    switch (goal) {
      case 'local':
        void openFolder();
        break;
      case 'vps':
        useOverlayStore.getState().openAddHost();
        break;
      case 'new':
        useOverlayStore.getState().openNewProject();
        break;
      case 'wordpress':
        useKernelUi.getState().openAgency('sites');
        break;
    }
  };

  const steps = k.steps;

  return (
    <Modal open={open && !covered} label={k.title} onClose={finish} center className="flex max-h-[92vh] w-[min(640px,96vw)] flex-col overflow-hidden">
      <div className="border-b border-border-subtle px-[24px] py-[16px]">
        <h2 className="flex items-center gap-[8px] text-[16px] font-semibold text-text-primary">
          <BrandLogo size={22} glow />
          {k.title}
        </h2>
        <ol className="mt-[10px] flex gap-[6px]" aria-label={k.progress}>
          {steps.map((label, index) => (
            <li
              key={label}
              aria-current={index === step}
              className={'h-[4px] flex-1 rounded-full ' + (index <= step ? 'bg-accent' : 'bg-bg-hover')}
              title={label}
            />
          ))}
        </ol>
        <p className="mt-[6px] text-[11.5px] text-text-secondary">{k.stepOf(step + 1, steps.length, steps[step] ?? '')}</p>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-[24px] py-[18px] text-[13px]">
        {step === 0 ? (
          <div className="grid grid-cols-2 gap-[10px]">
            {GOALS.map((item) => {
              const Icon = item.icon;

              return (
                <button
                  key={item.id}
                  type="button"
                  aria-pressed={goal === item.id}
                  className={'flex flex-col items-start gap-[6px] rounded-lg border p-[14px] text-left transition-colors ' + (goal === item.id ? 'border-border-focus bg-accent-subtle' : 'border-border-subtle bg-bg-raised hover:border-border-strong')}
                  onClick={() => setGoal(item.id)}
                >
                  <Icon size={18} className="text-accent" aria-hidden="true" />
                  <span className="font-semibold text-text-primary">{k.goal[item.id]}</span>
                  <span className="text-[11.5px] text-text-secondary">{k.goalHelp[item.id]}</span>
                </button>
              );
            })}
          </div>
        ) : null}

        {step === 1 ? (
          <div className="flex flex-col gap-[12px]">
            <p className="text-text-secondary">{k.modelHelp}</p>
            <p className={providers > 0 ? 'text-state-success' : 'text-state-waiting'}>{providers > 0 ? k.connected(providers) : k.noneConnected}</p>
            <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' self-start'} onClick={() => useOverlayStore.getState().openHub()}>
              <Plug size={12} aria-hidden="true" />
              {k.openHub}
            </button>
          </div>
        ) : null}

        {step === 2 ? (
          <div className="flex flex-col gap-[10px]">
            <p className="text-text-secondary">{k.doctorHelp}</p>
            <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' self-start'} onClick={() => void runDoctor('local')}>
              <Stethoscope size={12} aria-hidden="true" />
              {k.runDoctor}
            </button>
            <ul className="flex flex-col gap-[4px] text-[12px]">
              {doctor.map((check) => (
                <li key={check.id} className={check.state === 'ok' ? 'text-text-secondary' : check.state === 'warn' ? 'text-state-waiting' : 'text-state-error'}>
                  {check.state === 'ok' ? '✓' : check.state === 'warn' ? '!' : '✗'} {check.label} - {check.detail}
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        {step === 3 ? (
          <ul className="flex flex-col gap-[12px]">
            {([
              [History, k.safety.timeMachine],
              [Check, k.safety.contract],
              [OctagonX, k.safety.kill],
            ] as const).map(([Icon, text]) => (
              <li key={text} className="flex gap-[10px]">
                <Icon size={16} className="mt-[2px] shrink-0 text-accent" aria-hidden="true" />
                <span className="text-text-secondary">{text}</span>
              </li>
            ))}
            <li className="rounded-md bg-bg-raised px-[12px] py-[8px] text-[12px] text-text-secondary">{k.safety.demo}</li>
          </ul>
        ) : null}

        {step === 4 ? (
          <div className="flex flex-col gap-[10px]">
            <p className="text-text-secondary">{k.firstTaskHelp[goal]}</p>
            <button type="button" className={BTN_LG + ' ' + BTN_PRIMARY + ' self-start'} onClick={firstTask}>
              {k.firstTask[goal]}
            </button>
          </div>
        ) : null}
      </div>

      <div className="flex justify-between gap-[8px] border-t border-border-subtle px-[24px] py-[12px]">
        <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={finish}>
          {k.skip}
        </button>
        <div className="flex gap-[8px]">
          {step > 0 ? (
            <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={() => setStep(step - 1)}>
              {k.back}
            </button>
          ) : null}
          {step < steps.length - 1 ? (
            <button type="button" className={BTN + ' ' + BTN_PRIMARY} onClick={() => setStep(step + 1)}>
              {k.next}
            </button>
          ) : null}
        </div>
      </div>
    </Modal>
  );
}
