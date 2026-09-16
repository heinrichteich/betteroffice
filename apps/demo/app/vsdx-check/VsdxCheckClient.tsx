"use client";

import dynamic from "next/dynamic";
import { useEffect, useState } from "react";
import type { VsdxFontFace } from "@betteroffice/vsdx";
import { loadBundledFontBytes, resolveLastResortFace, resolveMetricCompatFace } from "@betteroffice/fonts";

const VsdxEditor = dynamic(
  () => import("@betteroffice/vsdx-react").then((module) => module.VsdxEditor),
  { ssr: false },
);

export function VsdxCheckClient() {
  const [assets, setAssets] = useState<{ file: Uint8Array; fonts: VsdxFontFace[] } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([loadDiagram(), loadDiagramFonts()]).then(
      ([file, fonts]) => { if (!cancelled) setAssets({ file, fonts }); },
      (value: unknown) => { if (!cancelled) setError(value instanceof Error ? value.message : String(value)); },
    );
    return () => { cancelled = true; };
  }, []);

  return (
    <main data-testid="vsdx-check-stage" style={{ position: "fixed", inset: 0 }}>
      {error ? <p role="alert">{error}</p> : assets ? (
        <VsdxEditor
          file={assets.file}
          fonts={assets.fonts}
          onReady={(api) => { (window as unknown as { __vsdxCheck: unknown }).__vsdxCheck = api; }}
        />
      ) : <p>Loading diagram…</p>}
    </main>
  );
}

async function loadDiagram(): Promise<Uint8Array> {
  const response = await fetch("/vsdx-check-connector.vsdx");
  if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function loadDiagramFonts(): Promise<VsdxFontFace[]> {
  const face = resolveMetricCompatFace("Arial", false, false) ?? resolveLastResortFace("Arial", false, false);
  const bytes = await loadBundledFontBytes(face);
  return [{ family: "Arial", bytes: new Uint8Array(bytes.slice(0)) }];
}
