/**
 * Dropping PDFs onto the window (SPEC 11.3).
 *
 * Tauri, not the DOM. A webview's own drag-and-drop hands over a `File` object
 * with no path — the browser sandbox is explicit about never revealing one —
 * and the backend opens documents *by path*, because a worker has to open the
 * file itself. Tauri's window-level drag-drop event is the one that carries
 * real paths, which is why `dragDropEnabled` is set in `tauri.conf.json` and
 * why this listens there instead of on `document`.
 */

import { useEffect, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useWorkspace } from "@/state/workspaceStore";

/** Payloads Tauri sends on the window's drag-drop channel. */
interface DragDropPayload {
  readonly type: "enter" | "over" | "drop" | "leave";
  readonly paths?: string[];
}

function isPdf(path: string): boolean {
  return path.toLowerCase().endsWith(".pdf");
}

/**
 * Listens for dropped files and opens the PDFs among them.
 *
 * Returns whether something is currently hovering, for the drop affordance.
 */
export function useDropTarget(): boolean {
  const [over, setOver] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void getCurrentWebviewWindow()
      .onDragDropEvent((event) => {
        const payload = event.payload as DragDropPayload;
        if (payload.type === "drop") {
          setOver(false);
          for (const path of (payload.paths ?? []).filter(isPdf)) {
            void useWorkspace.getState().openFile(path);
          }
        } else if (payload.type === "leave") {
          setOver(false);
        } else {
          setOver(true);
        }
      })
      .then((fn) => {
        // The window can go away while the listener is being registered; the
        // handle then has to be dropped rather than stored.
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // No Tauri host — the browser dev server. Dropping is simply absent
        // there rather than broken.
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return over;
}
