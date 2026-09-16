import { describe, expect, test } from 'bun:test';
import { MAX_TIFF_BYTES } from '../../../../shared/media';
import { maybeTranscodeTiffMedia } from './tiffMedia';

const PNG_BASE64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
const PNG_MAGIC = [137, 80, 78, 71, 13, 10, 26, 10];

function pngBytes(): Uint8Array {
  const binary = atob(PNG_BASE64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

/** Minimal single-strip little-endian 2x1 RGBA TIFF, uncompressed. */
function tinyRgbaTiff(): Uint8Array {
  const pixels = [255, 0, 0, 255, 0, 128, 255, 64];
  const entries: Array<[number, number, number, number]> = [
    [256, 3, 1, 2],
    [257, 3, 1, 1],
    [258, 3, 4, 122],
    [259, 3, 1, 1],
    [262, 3, 1, 2],
    [273, 4, 1, 130],
    [277, 3, 1, 4],
    [278, 4, 1, 1],
    [279, 4, 1, 8],
  ];
  const tiff = new Uint8Array(138);
  const view = new DataView(tiff.buffer);
  view.setUint16(0, 0x4949, true);
  view.setUint16(2, 42, true);
  view.setUint32(4, 8, true);
  view.setUint16(8, entries.length, true);
  entries.forEach(([tag, type, count, value], index) => {
    const offset = 10 + index * 12;
    view.setUint16(offset, tag, true);
    view.setUint16(offset + 2, type, true);
    view.setUint32(offset + 4, count, true);
    view.setUint32(offset + 8, value, true);
  });
  for (let index = 0; index < 4; index += 1) view.setUint16(122 + index * 2, 8, true);
  tiff.set(pixels, 130);
  return tiff;
}

function failingTranscode(): (bytes: Uint8Array) => Uint8Array {
  return () => {
    throw new Error('transcode must not run');
  };
}

describe('maybeTranscodeTiffMedia', () => {
  test('passes non-TIFF media through with mime and dataUrl intact', () => {
    const bytes = pngBytes();
    const dataUrl = `data:image/png;base64,${PNG_BASE64}`;
    expect(maybeTranscodeTiffMedia(bytes, 'image/png', dataUrl, failingTranscode())).toEqual({
      bytes,
      mimeType: 'image/png',
      dataUrl,
    });
  });

  test('rejects TIFF past the transfer budget before transcoding', () => {
    const bytes = new Uint8Array(MAX_TIFF_BYTES + 1);
    bytes.set([0x4d, 0x4d, 0, 0x2a]);
    expect(() =>
      maybeTranscodeTiffMedia(bytes, 'image/tiff', 'data:image/tiff;base64,', failingTranscode())
    ).toThrow('TIFF image exceeds the browser transfer budget');
  });

  test('transcodes TIFF media to PNG bytes', () => {
    const tiff = tinyRgbaTiff();
    const seen: Uint8Array[] = [];
    const result = maybeTranscodeTiffMedia(
      tiff,
      'image/tiff',
      'data:image/tiff;base64,',
      (bytes) => {
        seen.push(bytes.slice());
        return pngBytes();
      }
    );
    expect(seen).toHaveLength(1);
    expect(seen[0]).toEqual(tiff);
    expect(Array.from(result.bytes.subarray(0, 8))).toEqual(PNG_MAGIC);
    expect(result.mimeType).toBe('image/png');
    expect(result.dataUrl).toBe(`data:image/png;base64,${PNG_BASE64}`);
  });
});

let facade: typeof import('./rustParseFacade') | undefined;
try {
  facade = await import('./rustParseFacade');
} catch {
  facade = undefined;
}
const describeIfWasm = facade ? describe : describe.skip;

function mediaEntry(
  alias: string,
  path: string,
  mimeType: string,
  bytes: Uint8Array
): [string, Record<string, string>] {
  const base64 = Buffer.from(bytes).toString('base64');
  return [alias, { path, mimeType, base64, dataUrl: `data:${mimeType};base64,${base64}` }];
}

function envelopeWithMedia(mediaEntries: unknown[]): unknown {
  return {
    wireVersion: 1,
    document: {
      package: {
        document: { content: [] },
        theme: {},
        numbering: {},
        settings: {},
        fontTable: {},
        relationshipEntries: [],
        mediaEntries,
        chartEntries: [],
      },
    },
    embeddedFontParts: [],
  };
}

function pngMagicOf(dataUrl: string): number[] {
  const base64 = dataUrl.replace('data:image/png;base64,', '');
  const binary = atob(base64);
  return Array.from(binary.slice(0, 8)).map((char) => char.charCodeAt(0));
}

describeIfWasm('decodeS9EnvelopeValue TIFF media', () => {
  test('transcodes shared TIFF sources to PNG and leaves other media alone', () => {
    if (!facade) throw new Error('parse wasm is unavailable');
    const tiff = tinyRgbaTiff();
    const png = pngBytes();
    const result = facade.decodeS9EnvelopeValue(
      envelopeWithMedia([
        mediaEntry('word/media/image1.tiff', 'word/media/image1.tiff', 'image/tiff', tiff),
        mediaEntry('media/image1.tiff', 'word/media/image1.tiff', 'image/tiff', tiff),
        mediaEntry('word/media/image2.png', 'word/media/image2.png', 'image/png', png),
      ]),
      new ArrayBuffer(0)
    );
    const media = result.document.package.media;
    expect(media).toBeDefined();
    if (!media) throw new Error('media must be decoded');
    const first = media.get('word/media/image1.tiff');
    const second = media.get('media/image1.tiff');
    expect(first).toBeDefined();
    expect(first).toBe(second);
    expect(first?.mimeType).toBe('image/png');
    expect((first?.dataUrl ?? '').startsWith('data:image/png;base64,')).toBe(true);
    expect(first ? pngMagicOf(first.dataUrl ?? '') : []).toEqual(PNG_MAGIC);
    expect(first ? Array.from(new Uint8Array(first.data).subarray(0, 8)) : []).toEqual(PNG_MAGIC);
    const passthrough = media.get('word/media/image2.png');
    expect(passthrough?.mimeType).toBe('image/png');
    expect(passthrough ? Array.from(new Uint8Array(passthrough.data)) : []).toEqual(
      Array.from(png)
    );
  });
});
