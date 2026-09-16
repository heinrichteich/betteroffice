import { Suspense } from "react";
import { VsdxCheckClient } from "./VsdxCheckClient";

export default function VsdxCheck() {
  return (
    <Suspense fallback={null}>
      <VsdxCheckClient />
    </Suspense>
  );
}
