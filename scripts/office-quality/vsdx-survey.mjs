import { execFile } from 'node:child_process';
import { mkdir, readdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { summarizeError } from './results.mjs';

export const COVERAGE_SCHEMA_VERSION = 1;
export const COVERAGE_PRESENTATION_LIMIT = 10;

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '..', '..');
const syntheticDir = join(root, 'crates', 'vsdx-parse', 'tests', 'fixtures');

const execute = promisify(execFile);

function isVsdxFile(name) {
  return name.endsWith('.vsdx') && !name.includes('/');
}

export async function listSyntheticFixtures() {
  const entries = await readdir(syntheticDir);
  return entries
    .filter(isVsdxFile)
    .sort()
    .map((name) => ({ name: name.slice(0, -5), path: join(syntheticDir, name) }));
}

export async function listCorpusDirFixtures(directory) {
  const entries = await readdir(directory);
  return entries
    .filter(isVsdxFile)
    .sort()
    .map((name) => ({ name: name.slice(0, -5), path: resolve(directory, name) }));
}

export async function collectSurveyInputs(environment = process.env) {
  const corpusDir = environment.VSDX_CORPUS_DIR?.trim() || environment.VSDX_EXPLORE_DIR?.trim();
  if (corpusDir) return listCorpusDirFixtures(corpusDir);
  if (environment.VSDX_SURVEY_DIR?.trim())
    return listCorpusDirFixtures(resolve(environment.VSDX_SURVEY_DIR.trim()));
  return listSyntheticFixtures();
}

export function aggregateCoverage(files) {
  const totals = {
    files: files.length,
    ok: 0,
    failed: 0,
    pages: 0,
    shapes: 0,
    primitives: 0,
    placeholders: 0,
    evaluated: 0,
    unsupported: 0,
    evalErrors: 0,
    fidelity: 0,
    integrity: 0,
  };
  const histogram = new Map();
  for (const file of files) {
    if (file.status !== 'ok') {
      totals.failed += 1;
      continue;
    }
    totals.ok += 1;
    totals.pages += file.pages ?? 0;
    totals.shapes += file.shapes ?? 0;
    totals.primitives += file.primitives ?? 0;
    totals.placeholders += file.placeholders ?? 0;
    totals.evaluated += file.evaluated ?? 0;
    totals.unsupported += file.unsupported ?? 0;
    totals.evalErrors += file.evalErrors ?? file.eval_errors ?? 0;
    totals.fidelity += file.fidelity ?? 0;
    totals.integrity += file.integrity ?? 0;
    for (const [code, count] of Object.entries(file.diagnostics ?? {}))
      histogram.set(code, (histogram.get(code) ?? 0) + count);
  }
  const diagnosticTotal = [...histogram.values()].reduce((sum, count) => sum + count, 0);
  return { totals, histogram, diagnosticTotal };
}

export function renderCoverageSection(report, limit = COVERAGE_PRESENTATION_LIMIT) {
  if (!/^[a-f0-9]{40}$/.test(report.commit ?? ''))
    throw new Error('Expected a full commit SHA');
  const { totals, histogram, diagnosticTotal } = aggregateCoverage(report.files);
  const ranked = [...histogram.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
  const shown = ranked.slice(0, limit);
  const rows = shown.map(([code, count]) => `| ${code} | ${count} |`).join('\n');
  const omitted = diagnosticTotal - shown.reduce((sum, [, count]) => sum + count, 0);
  return [
    '<!-- BEGIN GENERATED VSDX COVERAGE -->',
    '## VSDX engine coverage',
    '',
    'Engine coverage diagnostics count what the engine resolved. They are not visual fidelity scores, which need Visio reference images.',
    '',
    `Files ${totals.ok}/${totals.files} ok; pages ${totals.pages}; shapes ${totals.shapes}; primitives ${totals.primitives}; placeholders ${totals.placeholders}.`,
    `Evaluated ${totals.evaluated}; unsupported ${totals.unsupported}; errors ${totals.evalErrors}.`,
    `Render diagnostics ${diagnosticTotal} (fidelity ${totals.fidelity}, integrity ${totals.integrity}).`,
    '',
    '| Diagnostic | Count |',
    '| --- | ---: |',
    rows || '| — | 0 |',
    omitted > 0 ? `\n…and ${omitted} more across ${ranked.length - shown.length} codes.` : '',
    '',
    `Commit [${report.commit.slice(0, 8)}](https://github.com/openooxml/betteroffice/commit/${report.commit}).`,
    '<!-- END GENERATED VSDX COVERAGE -->',
  ]
    .filter((line) => line !== '')
    .join('\n');
}

export async function surveyWithBinary(inputs) {
  const args = inputs.map(({ name, path }) => `${name}=${path}`);
  const { stdout } = await execute(
    'cargo',
    ['run', '--quiet', '-p', 'betteroffice-vsdx-survey', '--', ...args],
    { cwd: root, timeout: 600_000, maxBuffer: 64 * 1024 * 1024 }
  );
  const output = JSON.parse(stdout);
  if (output?.schema_version !== 1 || !Array.isArray(output.files))
    throw new Error('Invalid VSDX survey output');
  return output.files;
}

export async function measureCoverage(inputs, survey = surveyWithBinary, log = () => {}) {
  const files = [];
  const runOne = async (input) => {
    try {
      const [result] = await survey([input]);
      if (!result || result.file !== input.name) throw new Error('Missing survey result');
      files.push({ ...result, name: input.name });
      log(`${input.name}: ${result.status}`);
    } catch (error) {
      files.push({
        file: input.name,
        name: input.name,
        status: 'failed',
        error: summarizeError(error),
      });
      log(`${input.name}: FAILED: ${summarizeError(error)}`);
    }
  };
  for (const input of inputs) await runOne(input);
  return files;
}

export async function runCoverage({
  environment = process.env,
  survey,
  outputDir,
  log = console.log,
} = {}) {
  const inputs = await collectSurveyInputs(environment);
  const commit = (
    await execute('git', ['log', '-1', '--format=%H'], { cwd: root })
  ).stdout.trim();
  const files = await measureCoverage(inputs, survey, log);
  const report = {
    schema_version: COVERAGE_SCHEMA_VERSION,
    commit,
    totals_note: 'Engine coverage only; visual fidelity needs Visio references.',
    files,
  };
  const directory = resolve(outputDir ?? environment.QUALITY_OUTPUT ?? join(root, '.source', 'office-quality', 'vsdx-coverage'));
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, 'coverage.json'), `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(join(directory, 'coverage.md'), `${renderCoverageSection(report)}\n`);
  return report;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  await runCoverage();
}
