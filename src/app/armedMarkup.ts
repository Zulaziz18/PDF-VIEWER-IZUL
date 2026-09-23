/**
 * Applies an armed text-markup tool to each selection the user makes.
 *
 * `markupSelection` arms the tool when the ribbon button is pressed with
 * nothing selected; this listener is the other half. It waits for the pointer
 * to be released — the moment a drag-selection is finished — and marks up
 * whatever is selected then. The tool stays armed until it is pressed again,
 * another tool is picked, or Escape is pressed.
 */

import { useEffect } from "react";
import { useUi } from "@/state/uiStore";
import { applyMarkup } from "./actions";

export function useArmedMarkup(): void {
  useEffect(() => {
    const onUp = (): void => {
      const armed = useUi.getState().markup;
      if (armed === null) return;
      // After the browser has finished extending the selection for this
      // release, not before it.
      window.setTimeout(() => {
        const selection = document.getSelection();
        if (!selection || selection.isCollapsed) return;
        if (useUi.getState().markup === armed) applyMarkup(armed);
      }, 0);
    };
    window.addEventListener("pointerup", onUp);
    return () => window.removeEventListener("pointerup", onUp);
  }, []);
}
