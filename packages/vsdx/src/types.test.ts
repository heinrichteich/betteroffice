import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

/** Guards the public surface against a duplicated declaration. */
test('TextReceipt is declared once', () => {
  const source = readFileSync(resolve(import.meta.dir, './types.ts'), 'utf8');
  expect(source.match(/export interface TextReceipt/g)).toHaveLength(1);
});
