import { useEffect, useState } from 'react';

import type { CliSelfCheck, CrashReport, GlossaryTerm, TeamMember, TeamState, UpdateInfo } from '../../../protocol/types';
import { chooseLocale, currentLocale, LOCALES, type Locale } from '../i18n';
import { openOutside } from '../lib/external';
import { storedSettings, storeSetting } from '../lib/settings';
import { strings } from '../strings';
import {
  checkUpdates,
  clearCrashReports,
  cliSelfCheck,
  crashReports,
  daemonSetting,
  loadGlossary,
  loadTeam,
  saveGlossaryTerm,
  saveTeam,
  shareStatus,
  voiceStatus,
} from '../store/kernelIntents';
import { useSessionsStore } from '../store/sessions';
import { Badge } from '../panels/ui/Badge';
import { BTN, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

/**
 * The three Settings tabs 0.12 adds: **Language** (the window's language, how answers come back, when the
 * Intent Contract card appears, voice, and the glossary SDC has learned), **Team** (who is at the desk and
 * what their role lets them do), and **Updates** (the channel, the release to roll back to, crash reports,
 * the CLI self-check, and the phone's status page).
 */
const inputClass = 'rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] text-[12px] text-text-primary';

function Head({ title, desc }: { title: string; desc: string }) {
  return (
    <div className="mb-[18px]">
      <h2 className="text-[17px] font-semibold text-text-primary">{title}</h2>
      <p className="mt-[4px] text-[12.5px] text-text-muted">{desc}</p>
    </div>
  );
}

function Row({ label, help, children }: { label: string; help?: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-[16px] border-b border-border-subtle py-[12px]">
      <div className="min-w-0">
        <div className="text-[12.5px] font-medium text-text-primary">{label}</div>
        {help === undefined ? null : <div className="mt-[2px] text-[11.5px] text-text-muted">{help}</div>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

export function LanguageTab() {
  const k = strings.kernel.language;
  const { activeTab } = useSessionsStore();
  const [contract, setContract] = useState<string>(() => String(storedSettings()['intent-contract'] ?? 'off'));
  const [style, setStyle] = useState<string>(() => String(storedSettings()['reply-style'] ?? 'standard'));
  const [glossary, setGlossary] = useState<{ scope: string; terms: GlossaryTerm[] } | null>(null);
  const [term, setTerm] = useState('');
  const [meaning, setMeaning] = useState('');
  const [voice, setVoice] = useState<Awaited<ReturnType<typeof voiceStatus>>>(null);

  useEffect(() => {
    void loadGlossary(activeTab).then(setGlossary);
    void voiceStatus().then(setVoice);
  }, [activeTab]);

  return (
    <div>
      <Head title={k.title} desc={k.desc} />
      <Row label={k.ui} help={k.uiHelp}>
        <select
          className={inputClass}
          value={currentLocale()}
          onChange={(event) => {
            const locale = event.target.value as Locale;

            /* Alerts, proof packs and approval pages are written by the daemon: it hears the choice too. */
            void daemonSetting('ui.language', locale).finally(() => chooseLocale(locale));
          }}
        >
          {LOCALES.map((locale) => (
            <option key={locale.id} value={locale.id}>
              {locale.name}
            </option>
          ))}
        </select>
      </Row>
      <Row label={k.contract} help={k.contractHelp}>
        <select
          className={inputClass}
          value={contract}
          onChange={(event) => {
            setContract(event.target.value);
            storeSetting('intent-contract', event.target.value);
          }}
        >
          {(['auto', 'always', 'off'] as const).map((mode) => (
            <option key={mode} value={mode}>
              {k.contractMode[mode]}
            </option>
          ))}
        </select>
      </Row>
      <Row label={k.style} help={k.styleHelp}>
        <select
          className={inputClass}
          value={style}
          onChange={(event) => {
            setStyle(event.target.value);
            storeSetting('reply-style', event.target.value);
            void daemonSetting('intent.replyStyle', event.target.value);
          }}
        >
          <option value="standard">{k.styleStandard}</option>
          <option value="dialect">{k.styleDialect}</option>
        </select>
      </Row>
      <Row label={k.code} help={k.codeHelp}>
        <Badge tone="muted">English</Badge>
      </Row>
      <Row label={k.voice} help={voice === null ? k.voiceUnknown : voice.available ? (voice.local ? k.voiceLocal(voice.program ?? '') : k.voiceOnline(voice.online.join(', '))) : (voice.hint ?? k.voiceNone)}>
        <Badge tone={voice?.available === true ? 'success' : 'warning'}>{voice?.available === true ? k.ready : k.notReady}</Badge>
      </Row>

      <div className="mt-[18px]">
        <div className="text-[12.5px] font-medium text-text-primary">{k.glossary}</div>
        <p className="mt-[2px] text-[11.5px] text-text-muted">{k.glossaryHelp(glossary?.scope ?? 'global')}</p>
        <ul className="mt-[8px] flex flex-col gap-[4px]">
          {(glossary?.terms ?? []).map((entry) => (
            <li key={`${entry.scope}:${entry.term}`} className="flex items-center gap-[8px] text-[12px]">
              <span className="font-mono">{entry.term}</span>
              <span className="text-text-muted">=</span>
              <span className="flex-1">{entry.meaning}</span>
              <button
                type="button"
                className={BTN_SM + ' ' + BTN_SECONDARY}
                onClick={() => void saveGlossaryTerm(entry.scope, entry.term, '').then((terms) => setGlossary(terms === null ? glossary : { scope: glossary?.scope ?? entry.scope, terms }))}
              >
                {k.forget}
              </button>
            </li>
          ))}
        </ul>
        <div className="mt-[8px] flex flex-wrap items-center gap-[6px]">
          <input className={inputClass + ' w-[140px]'} placeholder={strings.kernel.intent.term} value={term} onChange={(event) => setTerm(event.target.value)} />
          <span className="text-text-muted">=</span>
          <input className={inputClass + ' w-[200px]'} placeholder={strings.kernel.intent.meaning} value={meaning} onChange={(event) => setMeaning(event.target.value)} />
          <button
            type="button"
            className={BTN_SM + ' ' + BTN_SECONDARY}
            disabled={term.trim() === '' || meaning.trim() === '' || glossary === null}
            onClick={() =>
              void saveGlossaryTerm(glossary?.scope ?? 'global', term, meaning).then((terms) => {
                if (terms !== null) {
                  setGlossary({ scope: glossary?.scope ?? 'global', terms });
                  setTerm('');
                  setMeaning('');
                }
              })
            }
          >
            {strings.kernel.intent.addTerm}
          </button>
        </div>
      </div>
    </div>
  );
}

export function TeamTab() {
  const k = strings.kernel.team;
  const [team, setTeam] = useState<TeamState | null>(null);
  const [members, setMembers] = useState<TeamMember[]>([]);
  const [name, setName] = useState('');
  const [role, setRole] = useState<TeamMember['role']>('developer');
  const [email, setEmail] = useState(() => String(storedSettings()['agency-email'] ?? ''));
  const [guide, setGuide] = useState(() => String(storedSettings()['agency-style-guide'] ?? ''));

  useEffect(() => {
    void loadTeam().then((loaded) => {
      setTeam(loaded);
      setMembers(loaded?.members ?? []);
    });
  }, []);

  return (
    <div>
      <Head title={k.title} desc={k.desc} />
      <p className="mb-[12px] rounded-md bg-bg-raised px-[10px] py-[8px] text-[11.5px] text-text-muted">{k.localNote}</p>
      <Row label={k.atDesk} help={k.role(team?.role ?? 'owner')}>
        <select
          className={inputClass}
          value={team?.current ?? ''}
          onChange={(event) => void saveTeam({ current: event.target.value }).then(setTeam)}
        >
          <option value="">{k.nobody}</option>
          {members.map((member) => (
            <option key={member.name} value={member.name}>
              {member.name}
            </option>
          ))}
        </select>
      </Row>
      <ul className="mt-[12px] flex flex-col gap-[6px]">
        {members.map((member, index) => (
          <li key={member.name} className="flex items-center gap-[8px] text-[12.5px]">
            <span className="flex-1">{member.name}</span>
            <select
              className={inputClass}
              value={member.role}
              onChange={(event) => setMembers(members.map((entry, at) => (at === index ? { ...entry, role: event.target.value as TeamMember['role'] } : entry)))}
            >
              {(['owner', 'developer', 'reviewer', 'client'] as const).map((option) => (
                <option key={option} value={option}>
                  {k.roles[option]}
                </option>
              ))}
            </select>
            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => setMembers(members.filter((_, at) => at !== index))}>
              {k.remove}
            </button>
          </li>
        ))}
      </ul>
      <div className="mt-[10px] flex flex-wrap items-center gap-[6px]">
        <input className={inputClass} placeholder={k.name} value={name} onChange={(event) => setName(event.target.value)} />
        <select className={inputClass} value={role} onChange={(event) => setRole(event.target.value as TeamMember['role'])}>
          {(['owner', 'developer', 'reviewer', 'client'] as const).map((option) => (
            <option key={option} value={option}>
              {k.roles[option]}
            </option>
          ))}
        </select>
        <button
          type="button"
          className={BTN_SM + ' ' + BTN_SECONDARY}
          disabled={name.trim() === '' || members.some((member) => member.name === name.trim())}
          onClick={() => {
            setMembers([...members, { name: name.trim(), role }]);
            setName('');
          }}
        >
          {k.add}
        </button>
      </div>
      <ul className="mt-[10px] text-[11.5px] text-text-muted">
        {(['owner', 'developer', 'reviewer', 'client'] as const).map((option) => (
          <li key={option}>
            <span className="font-semibold text-text-secondary">{k.roles[option]}:</span> {k.roleHelp[option]}
          </li>
        ))}
      </ul>
      <button type="button" className={BTN + ' ' + BTN_PRIMARY + ' mt-[12px]'} onClick={() => void saveTeam({ members }).then((saved) => (saved === null ? undefined : setTeam(saved)))}>
        {k.save}
      </button>

      <div className="mt-[22px] flex flex-col gap-[10px]">
        <label className="flex flex-col gap-[4px] text-[12.5px] font-medium text-text-primary">
          {k.email}
          <span className="text-[11.5px] font-normal text-text-muted">{k.emailHelp}</span>
          <input
            className={inputClass}
            type="email"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
            onBlur={() => {
              storeSetting('agency-email', email);
              void daemonSetting('agency.email', email);
            }}
          />
        </label>
        <label className="flex flex-col gap-[4px] text-[12.5px] font-medium text-text-primary">
          {k.styleGuide}
          <span className="text-[11.5px] font-normal text-text-muted">{k.styleGuideHelp}</span>
          <textarea
            rows={6}
            className={inputClass}
            value={guide}
            onChange={(event) => setGuide(event.target.value)}
            onBlur={() => {
              storeSetting('agency-style-guide', guide);
              void daemonSetting('agency.styleGuide', guide);
            }}
          />
        </label>
      </div>
    </div>
  );
}

export function UpdatesTab() {
  const k = strings.kernel.updates;
  const [channel, setChannel] = useState<'stable' | 'beta'>(() => (storedSettings()['update-channel'] === 'beta' ? 'beta' : 'stable'));
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [clis, setClis] = useState<CliSelfCheck[]>([]);
  const [crashes, setCrashes] = useState<CrashReport[]>([]);
  const [share, setShare] = useState<{ enabled: boolean; url?: string; error?: string } | null>(null);
  const [lowBandwidth, setLowBandwidth] = useState(() => storedSettings()['low-bandwidth'] === true);

  useEffect(() => {
    void cliSelfCheck().then(setClis);
    void crashReports().then(setCrashes);
  }, []);

  return (
    <div>
      <Head title={k.title} desc={k.desc} />
      <Row label={k.channel} help={k.channelHelp}>
        <select
          className={inputClass}
          value={channel}
          onChange={(event) => {
            const next = event.target.value === 'beta' ? 'beta' : 'stable';

            setChannel(next);
            storeSetting('update-channel', next);
            void daemonSetting('update.channel', next);
          }}
        >
          <option value="stable">{k.stable}</option>
          <option value="beta">{k.beta}</option>
        </select>
      </Row>
      <div className="py-[12px]">
        <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={() => void checkUpdates(channel).then(setInfo)}>
          {k.check}
        </button>
        {info === null ? null : (
          <div className="mt-[10px] flex flex-col gap-[6px] text-[12px]">
            <p className={info.updateAvailable ? 'text-state-waiting' : 'text-state-success'}>
              {info.updateAvailable && info.latest !== null ? k.available(info.latest.tag, info.current) : k.upToDate(info.current)}
            </p>
            {info.latest === null || !info.updateAvailable ? null : (
              <button type="button" className={BTN_SM + ' ' + BTN_PRIMARY + ' self-start'} onClick={() => void openOutside(info.latest?.url ?? '')}>
                {k.download(info.latest.tag)}
              </button>
            )}
            {info.previous === null ? null : (
              <p className="text-text-muted">
                {k.rollback(info.previous.tag)}{' '}
                <button type="button" className="text-accent hover:underline" onClick={() => void openOutside(info.previous?.url ?? '')}>
                  {k.openRelease}
                </button>
              </p>
            )}
          </div>
        )}
      </div>

      <Row label={k.lowBandwidth} help={k.lowBandwidthHelp}>
        <input
          type="checkbox"
          className="accent-[var(--accent)]"
          checked={lowBandwidth}
          onChange={(event) => {
            setLowBandwidth(event.target.checked);
            storeSetting('low-bandwidth', event.target.checked);
            void daemonSetting('net.lowBandwidth', event.target.checked ? 'on' : 'off');
          }}
        />
      </Row>

      <Row label={k.phone} help={share?.url ?? share?.error ?? k.phoneHelp}>
        <input type="checkbox" className="accent-[var(--accent)]" checked={share?.enabled === true} onChange={(event) => void shareStatus(event.target.checked).then(setShare)} />
      </Row>

      <div className="mt-[18px]">
        <div className="text-[12.5px] font-medium text-text-primary">{k.selfCheck}</div>
        <ul className="mt-[6px] flex flex-col gap-[4px] text-[12px]">
          {clis.map((cli) => (
            <li key={cli.program} className="flex items-center gap-[8px]">
              <Badge tone={cli.installed && cli.signedIn ? 'success' : cli.installed ? 'warning' : 'muted'}>{cli.label}</Badge>
              <span className="text-text-secondary">{cli.sentence}</span>
              {cli.version === null ? null : <span className="font-mono text-[10.5px] text-text-muted">{cli.version}</span>}
            </li>
          ))}
        </ul>
      </div>

      <div className="mt-[18px]">
        <div className="text-[12.5px] font-medium text-text-primary">{k.crashes}</div>
        <p className="mt-[2px] text-[11.5px] text-text-muted">{k.crashesHelp}</p>
        {crashes.length === 0 ? <p className="mt-[6px] text-[12px] text-state-success">{k.noCrashes}</p> : null}
        <ul className="mt-[6px] flex flex-col gap-[6px]">
          {crashes.map((crash) => (
            <li key={crash.file} className="rounded-md border border-border-subtle p-[8px] text-[11.5px]">
              <div className="font-mono text-text-muted">{crash.report.at}</div>
              <div className="text-text-primary">{crash.report.message}</div>
              <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY + ' mt-[6px]'} onClick={() => void openOutside(crash.issueUrl)}>
                {k.send}
              </button>
            </li>
          ))}
        </ul>
        {crashes.length === 0 ? null : (
          <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY + ' mt-[8px]'} onClick={() => void clearCrashReports().then(() => setCrashes([]))}>
            {k.clear}
          </button>
        )}
      </div>
    </div>
  );
}
