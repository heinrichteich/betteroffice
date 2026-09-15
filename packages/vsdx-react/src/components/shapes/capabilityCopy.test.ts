import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = join(import.meta.dir, '..', '..', '..', '..', '..');
const pkg = JSON.parse(readFileSync(join(ROOT, 'packages/vsdx-react/package.json'), 'utf8')) as { description: string };
const README = readFileSync(join(ROOT, 'README.md'), 'utf8');
const CONTENT = readFileSync(join(ROOT, 'apps/web/app/content.ts'), 'utf8');
const LLMS = readFileSync(join(ROOT, 'apps/web/public/llms.txt'), 'utf8');

describe('capability copy', () => {
  test('editor description owns the resize handles the canvas ships', () => {
    expect(pkg.description).toMatch(/resize with handles/);
    expect(pkg.description).not.toMatch(/no resize-handle/i);
  });

  test('stencil galleries are public copy everywhere', () => {
    for (const copy of [pkg.description, README, CONTENT, LLMS]) {
      expect(copy).toMatch(/three switchable stencils/);
      expect(copy).toMatch(/stencil browser/);
    }
  });
});
