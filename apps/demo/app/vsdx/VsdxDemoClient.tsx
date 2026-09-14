"use client";

import dynamic from "next/dynamic";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CollaborationProvider, initWasm, openDiagram, type CollaborationUser, type VsdxFontFace } from "@betteroffice/vsdx";
import { loadBundledFontBytes, resolveLastResortFace, resolveMetricCompatFace } from "@betteroffice/fonts";
import { Logo } from "../components/Logo";
import { CollaborationControls, COLLAB_RELAY_ORIGIN, useCollabRoom, useDemoRoom, useLeaveRoom, type CollaborationReplica, type CollaborationTransport } from "../collab";

const VsdxEditor = dynamic(
  () => import("@betteroffice/vsdx-react").then((module) => module.VsdxEditor),
  { ssr: false },
);

const SHOWCASE = {
  url: "/betteroffice-demo.vsdx",
  name: "betteroffice-demo.vsdx",
};

/** Loaded bytes plus the room seed; a local file carries no seed and stays private. */
interface DemoSource {
  id: number;
  file: Uint8Array;
  name: string;
  seed: Uint8Array | null;
}

export function VsdxDemoClient() {
  const [source, setSource] = useState<DemoSource | null>(null);
  const [fonts, setFonts] = useState<VsdxFontFace[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [dragging, setDragging] = useState(false);
  const openSequence = useRef(0);
  const dragDepth = useRef(0);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const room = useDemoRoom(source ? source.seed !== null : true);
  const leaveRoom = useLeaveRoom();
  const createProvider = useCallback((replica: CollaborationReplica, transport: CollaborationTransport) => new CollaborationProvider(replica, transport, { user: { name: presenceName() } }), []);
  const collab = useCollabRoom(COLLAB_RELAY_ORIGIN, room, createProvider);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([loadDiagram(), loadCollaborationSeed(), loadDiagramFonts()]).then(
      ([file, seed, loadedFonts]) => {
        if (cancelled || openSequence.current !== 0) return;
        setFonts(loadedFonts);
        setSource({ id: 0, file, name: SHOWCASE.name, seed });
      },
      (value: unknown) => { if (!cancelled) setLoadError(value instanceof Error ? value.message : String(value)); },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const openChosenFile = useCallback(async (picked: File) => {
    const sequence = openSequence.current + 1;
    openSequence.current = sequence;
    setOpening(true);
    try {
      const bytes = new Uint8Array(await picked.arrayBuffer());
      if (openSequence.current !== sequence) return;
      await ensureVisioOpenable(bytes);
      if (openSequence.current !== sequence) return;
      leaveRoom();
      setOpenError(null);
      setSource({ id: sequence, file: bytes, name: picked.name, seed: null });
    } catch (cause) {
      if (openSequence.current !== sequence) return;
      const reason = cause instanceof Error ? cause.message : String(cause);
      setOpenError(`Could not open \u201C${picked.name}\u201D: ${reason} Previous diagram kept.`);
    } finally {
      if (openSequence.current === sequence) setOpening(false);
    }
  }, [leaveRoom]);

  const onPickFiles = useCallback((list: FileList | null) => {
    const picked = list?.[0];
    if (!picked) return;
    void openChosenFile(picked);
  }, [openChosenFile]);

  const onDrop = useCallback((event: React.DragEvent) => {
    event.preventDefault();
    dragDepth.current = 0;
    setDragging(false);
    const picked = event.dataTransfer.files?.[0];
    if (picked) void openChosenFile(picked);
  }, [openChosenFile]);

  const collaboration = useMemo(() => {
    if (!source || source.seed === null) return undefined;
    if (!room || !collab.clientId) return undefined;
    return { clientId: collab.clientId, initialUpdate: source.seed, onReplica: collab.onReplica, presence: collab.provider ?? undefined };
  }, [collab.clientId, collab.onReplica, collab.provider, room, source]);

  const ready = Boolean(source && fonts && (source.seed === null || collaboration));
  const shared = source !== null && source.seed !== null;

  return (
    <div className="fixed inset-0 z-20 flex flex-col bg-surface text-fg">
      <header className="z-2 flex items-center gap-3.5 border-b border-hairline bg-white/92 px-4 py-[11px] backdrop-blur-lg">
        <div className="flex min-w-0 items-baseline gap-2.5">
          <Link href="/" className="inline-flex items-baseline gap-2 text-[14px] font-[650] tracking-[-0.01em] whitespace-nowrap text-fg no-underline">
            <Logo height={18} className="self-center" />
            BetterOffice <span className="font-normal text-faint">/ vsdx</span>
          </Link>
          <span className="overflow-hidden text-[12.5px] text-ellipsis whitespace-nowrap text-mute">In-browser Visio diagram editor</span>
        </div>
        <div className="flex-1" />
        {source && <span className="max-w-[180px] overflow-hidden text-[12.5px] text-ellipsis whitespace-nowrap text-mute">{source.name}</span>}
        <div className="flex flex-none items-center gap-2">
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            disabled={opening || !fonts}
            aria-label="Open a Visio file from your computer"
            className="inline-flex h-8 cursor-pointer items-center rounded-[5px] border border-hairline-strong bg-white px-[11px] text-[12.5px] text-fg transition-colors duration-[140ms] ease-[ease] hover:bg-surface disabled:cursor-default disabled:opacity-50"
          >
            {opening ? "Opening\u2026" : "Open file"}
          </button>
          <input
            ref={fileInputRef}
            type="file"
            accept=".vsdx,.vstx"
            aria-label="Choose a Visio file"
            data-testid="vsdx-file-input"
            style={{ display: "none" }}
            onChange={(event) => {
              onPickFiles(event.target.files);
              event.target.value = "";
            }}
          />
          <CollaborationControls status={collab.status} synced={collab.synced} peerCount={collab.peerCount} error={collab.error} shared={shared} />
          <a className="inline-flex size-8 items-center justify-center rounded-[5px] text-mute transition-colors duration-[140ms] ease-[ease] hover:bg-surface hover:text-fg" href="https://github.com/openooxml/betteroffice" target="_blank" rel="noreferrer" aria-label="View on GitHub" title="View on GitHub">
          <svg width="18" height="18" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" /></svg>
          </a>
        </div>
      </header>
      {openError && (
        <div className="z-2 flex items-center gap-3 border-b border-[#f3c7cf] bg-[#fdecef] px-4 py-2 text-[13px] text-danger" role="alert">
          <span>{openError}</span>
          <span className="flex-1" />
          <button type="button" onClick={() => setOpenError(null)} aria-label="Dismiss error" className="cursor-pointer rounded bg-transparent px-1.5 py-0.5 text-[16px] leading-none text-danger hover:bg-danger/10">
            {"\u00D7"}
          </button>
        </div>
      )}
      <main
        className="relative flex min-h-0 flex-1 flex-col *:min-h-0 *:flex-1"
        data-testid="vsdx-demo-stage"
        onDragEnter={(event) => {
          event.preventDefault();
          dragDepth.current += 1;
          setDragging(true);
        }}
        onDragOver={(event) => {
          event.preventDefault();
        }}
        onDragLeave={(event) => {
          event.preventDefault();
          dragDepth.current = Math.max(0, dragDepth.current - 1);
          if (dragDepth.current === 0) setDragging(false);
        }}
        onDrop={onDrop}
      >
        {loadError ? <p className="m-auto text-mute" role="alert">Failed to load the demo diagram: {loadError}</p> : source && fonts && ready ? <VsdxEditor key={`${room ?? "private"}:${source.id}`} file={source.file} fonts={fonts} collaboration={collaboration} /> : <p className="m-auto text-mute">Loading diagram…</p>}
        {dragging && (
          <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center bg-white/70 p-8" role="status">
            <div className="grid w-[min(440px,100%)] place-items-center rounded-md border-2 border-dashed border-acc bg-white px-8 py-10 text-center">
              <p className="mb-1 text-[16px] font-[650]">Drop to open the diagram</p>
              <p className="text-[13px] text-mute">Only .vsdx or .vstx, opened locally in your browser.</p>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

/** Probe local bytes with the wasm opener so a bad file never replaces the loaded diagram. */
async function ensureVisioOpenable(bytes: Uint8Array): Promise<void> {
  if (bytes.byteLength === 0) throw new Error("file is empty.");
  if (!looksLikeZip(bytes)) throw new Error("not a Visio package: missing ZIP header.");
  await initWasm();
  const probe = openDiagram(bytes.slice(), {});
  probe.dispose();
}

function looksLikeZip(bytes: Uint8Array): boolean {
  if (bytes.byteLength < 4) return false;
  return bytes[0] === 0x50 && bytes[1] === 0x4b && (bytes[2] === 0x03 || bytes[2] === 0x05 || bytes[2] === 0x07) && (bytes[3] === 0x04 || bytes[3] === 0x06 || bytes[3] === 0x08);
}

function presenceName(): CollaborationUser["name"] {
  return `VSDX ${crypto.getRandomValues(new Uint32Array(1))[0].toString(36)}`;
}

async function loadCollaborationSeed(): Promise<Uint8Array> {
  const response = await fetch("/seeds/vsdx.bin");
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function loadDiagram(): Promise<Uint8Array> {
  const response = await fetch(SHOWCASE.url);
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function loadDiagramFonts(): Promise<VsdxFontFace[]> {
  const styles = [
    { bold: false, italic: false },
    { bold: true, italic: false },
    { bold: false, italic: true },
    { bold: true, italic: true },
  ];
  return Promise.all(
    styles.map(async ({ bold, italic }) => {
      const face =
        resolveMetricCompatFace("Arial", bold, italic) ??
        resolveLastResortFace("Arial", bold, italic);
      const bytes = await loadBundledFontBytes(face);
      return {
        family: "Arial",
        bold,
        italic,
        bytes: new Uint8Array(bytes.slice(0)),
      };
    }),
  );
}
