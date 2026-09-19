import { expect, test } from 'bun:test';
import {
  aggregateCoverage,
  collectSurveyInputs,
  measureCoverage,
  renderCoverageSection,
} from './vsdx-survey.mjs';

const commit = 'c'.repeat(40);

function file(name: string, diagnostics: Record<string, number>, extra = {}) {
  return {
    file: name,
    name,
    status: 'ok',
    pages: 1,
    shapes: 2,
    primitives: 3,
    placeholders: 0,
    evaluated: 4,
    unsupported: 1,
    evalErrors: 0,
    fidelity: 0,
    integrity: 0,
    diagnostics,
    ...extra,
  };
}

test('synthetic fixtures run with an empty environment', async () => {
  const inputs = await collectSurveyInputs({});
  expect(inputs.length).toBeGreaterThan(0);
  expect(inputs.every((input) => input.path?.endsWith('.vsdx'))).toBe(true);
});

test('corpus selection reuses the shared sample selector', async () => {
  const inputs = await collectSurveyInputs(
    { QUALITY_SAMPLES: '["demo-vsdx"]' },
    async () => {
      throw new Error('Unexpected download');
    }
  );
  expect(inputs).toEqual([{ name: 'demo-vsdx', corpus: 'demo-vsdx' }]);
});

test('totals use the complete histogram before truncating the view', () => {
  const diagnostics: Record<string, number> = {};
  for (let index = 0; index < 20; index += 1) diagnostics[`code-${index}`] = index + 1;
  const report = { commit, files: [file('a', diagnostics)] };
  const { diagnosticTotal, histogram } = aggregateCoverage(report.files);
  expect(diagnosticTotal).toBe(210);
  expect(histogram.size).toBe(20);
  const section = renderCoverageSection(report, 5);
  expect(section).toContain('…and 120 more across 15 codes.');
  expect(section).toContain('Render diagnostics 210');
});

test('coverage stays distinct from visual fidelity scores', () => {
  const report = { commit, files: [file('a', { 'unresolved-x': 2 })] };
  const section = renderCoverageSection(report);
  expect(section).toContain('not visual fidelity scores');
  expect(section).not.toContain('SSIM');
  expect(section).toContain('BEGIN GENERATED VSDX COVERAGE');
});

test('survey failures reuse the shared failure reporter', async () => {
  const files = await measureCoverage(
    [{ name: 'broken', path: '/no/such/file.vsdx' }],
    async () => {
      throw Object.assign(new Error('Command failed'), {
        stderr: 'cargo: error: could not read /private/input.vsdx\n',
      });
    },
    () => {}
  );
  expect(files[0].status).toBe('failed');
  expect(files[0].error).not.toContain('/private/');
});
