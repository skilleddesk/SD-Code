// Dev-only: what the release workflow is doing, from the public API.
//   node sdc/scripts/tools/ci-status.mjs
import { execFileSync } from 'node:child_process';

const repo = 'skilleddesk/SD-Code';

function api(path) {
  try {
    const raw = execFileSync('curl.exe', ['-sS', `https://api.github.com/repos/${repo}${path}`], { encoding: 'utf8' });

    return { ok: true, body: JSON.parse(raw) };
  } catch (error) {
    return { ok: false, body: null, error: String(error) };
  }
}

const runs = api('/actions/runs?per_page=4');

if (!runs.ok || runs.body?.message) {
  console.log('The API did not answer (a private repository needs a token):');
  console.log(`  ${runs.body?.message ?? runs.error ?? 'no response'}`);
  console.log(`  Watch it at https://github.com/${repo}/actions`);
  process.exit(0);
}

for (const run of runs.body.workflow_runs ?? []) {
  console.log(`${run.name} · ${run.head_branch} · ${run.status}${run.conclusion ? ` / ${run.conclusion}` : ''}`);
  console.log(`  ${run.html_url}`);

  if (run.name === 'release') {
    const jobs = api(`/actions/runs/${run.id}/jobs`);

    for (const job of jobs.body?.jobs ?? []) {
      const step = (job.steps ?? []).find((candidate) => candidate.status === 'in_progress');

      console.log(`    ${job.name} · ${job.status}${job.conclusion ? ` / ${job.conclusion}` : ''}${step ? ` · now: ${step.name}` : ''}`);
    }
  }
}

const release = api('/releases/latest');

if (release.ok && !release.body?.message) {
  console.log(`\nlatest release: ${release.body.tag_name} — ${(release.body.assets ?? []).length} assets`);
  for (const asset of release.body.assets ?? []) {
    console.log(`  ${asset.name}  ${Math.round(asset.size / 1024 / 1024 * 10) / 10} MB`);
  }
} else {
  console.log('\nno release yet (or the repository is private)');
}
