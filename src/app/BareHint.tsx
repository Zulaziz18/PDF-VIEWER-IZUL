/**
 * What shows over the document in presentation and focus mode: how to get
 * out, for a few seconds — a mode with no visible way out is a trap (the
 * lesson of Phase 3's window without buttons) — and, while presenting, the
 * page number in a corner.
 */

import { useEffect, useState, type JSX } from "react";
import { fill } from "./fileActions";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { useUi } from "@/state/uiStore";

export function BareHint(props: { presenting: boolean }): JSX.Element {
  const page = useDocument((s) => s.page);
  const count = useDocument((s) => s.pageCount);
  const [shown, setShown] = useState(true);

  useEffect(() => {
    setShown(true);
    const id = window.setTimeout(() => setShown(false), 3000);
    return () => window.clearTimeout(id);
  }, [props.presenting]);

  return (
    <>
      <div
        role="status"
        className={[
          "fixed top-4 left-1/2 -translate-x-1/2 z-50 px-4 py-2 rounded-full text-[13px] bg-black/75 text-white",
          "transition-opacity duration-150 motion-reduce:transition-none",
          shown ? "opacity-100" : "opacity-0 pointer-events-none",
        ].join(" ")}
      >
        {props.presenting ? t("present.hint") : t("focus.hint")}
        <button
          type="button"
          onClick={() => {
            const ui = useUi.getState();
            if (props.presenting) ui.setPresenting(false);
            else ui.setFocusMode(false);
          }}
          className="ml-3 underline underline-offset-2"
        >
          {t("present.exit")}
        </button>
      </div>
      {props.presenting && (
        <div
          aria-live="polite"
          className="fixed bottom-4 right-5 z-50 px-2.5 py-1 rounded-[6px] text-[12px] bg-black/60 text-white/85 tabular-nums"
        >
          {fill(t("present.page"), { n: page + 1, count })}
        </div>
      )}
    </>
  );
}
