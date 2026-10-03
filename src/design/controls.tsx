/**
 * The primitives the ribbon, the bottom bar and the home screen are built from.
 *
 * Shaped after WPS Office's ribbon (SPEC 12, revised 2026-09-23): a large
 * button is an icon above a label, a small button is an icon beside a label and
 * two of them stack in the height of one large one, groups are separated by a
 * hairline, and a caret marks a button that opens a list of variants.
 *
 * Every control is a real `<button>` with an accessible name, so the whole
 * ribbon is reachable with Tab and readable by a screen reader (SPEC 14). No
 * control here does anything by itself; each takes the action it runs, which
 * lives in `src/app/actions.ts` or a store — never in the component (SPEC 0).
 */

import { useEffect, useId, useLayoutEffect, useRef, useState, type JSX, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName, type Tone } from "./Icon";

const HOVER = "hover:bg-[var(--izul-surface-raised)] active:bg-[var(--izul-chrome-hover)]";
const PRESSED = "bg-[var(--izul-accent-soft)] text-[var(--izul-text)]";
const MOTION = "transition-colors duration-[120ms] ease-[cubic-bezier(0.32,0.72,0,1)]";

export interface ActionProps {
  icon: IconName;
  tone?: Tone;
  label: string;
  /** Longer text for the tooltip, e.g. with the keyboard shortcut. */
  hint?: string;
  onClick: () => void;
  pressed?: boolean;
  disabled?: boolean;
}

/** Icon above label: the ribbon's primary button. */
export function RibbonButton(props: ActionProps & { menu?: MenuItem[] }): JSX.Element {
  const body = (
    <>
      <Icon name={props.icon} size={24} tone={props.tone ?? "blue"} />
      <span className="text-[12px] leading-[14px] whitespace-nowrap flex items-center gap-0.5">
        {props.label}
        {props.menu && <Icon name="chevronDown" size={16} className="-mr-1 opacity-70" />}
      </span>
    </>
  );
  const className = [
    "h-[60px] min-w-[52px] px-1.5 rounded-[6px] flex flex-col items-center justify-center gap-1",
    MOTION,
    props.pressed === true ? PRESSED : HOVER,
    "disabled:opacity-40 disabled:pointer-events-none",
  ].join(" ");
  if (props.menu) {
    return (
      <MenuButton label={props.label} hint={props.hint} items={props.menu} className={className}>
        {body}
      </MenuButton>
    );
  }
  return (
    <button
      type="button"
      onClick={props.onClick}
      disabled={props.disabled ?? false}
      aria-pressed={props.pressed}
      title={props.hint ?? props.label}
      className={className}
    >
      {body}
    </button>
  );
}

/** Icon beside label, half the height of a large button. */
export function RibbonSmall(props: ActionProps & { hideLabel?: boolean }): JSX.Element {
  return (
    <button
      type="button"
      onClick={props.onClick}
      disabled={props.disabled ?? false}
      aria-pressed={props.pressed}
      aria-label={props.hideLabel === true ? props.label : undefined}
      title={props.hint ?? props.label}
      className={[
        "h-[28px] px-1.5 rounded-[6px] flex items-center gap-1.5 text-[12px] whitespace-nowrap",
        MOTION,
        props.pressed === true ? PRESSED : HOVER,
        "disabled:opacity-40 disabled:pointer-events-none",
      ].join(" ")}
    >
      <Icon name={props.icon} size={20} tone={props.tone ?? "neutral"} />
      {/* Below ~1060 CSS px (a 1920 px screen at 175 %) the small buttons drop
          their labels, as WPS's ribbon does, so the ribbon still fits. The
          name stays in the tooltip and the accessible name. */}
      {props.hideLabel !== true && <span className="max-[1060px]:sr-only">{props.label}</span>}
    </button>
  );
}

/** Two small buttons stacked in the height of one large one. */
export function RibbonStack(props: { children: ReactNode }): JSX.Element {
  return <div className="flex flex-col justify-center gap-1">{props.children}</div>;
}

/** The hairline between groups. */
export function RibbonDivider(): JSX.Element {
  return <span aria-hidden="true" className="self-stretch w-px my-2 mx-1.5 bg-[var(--izul-border)]" />;
}

/** A bare icon button, for the title bar, bottom bar and panel headers. */
export function IconButton(props: {
  icon: IconName;
  label: string;
  hint?: string;
  onClick: () => void;
  tone?: Tone;
  size?: 16 | 20;
  pressed?: boolean;
  disabled?: boolean;
  className?: string;
}): JSX.Element {
  return (
    <button
      type="button"
      onClick={props.onClick}
      disabled={props.disabled ?? false}
      aria-label={props.label}
      aria-pressed={props.pressed}
      title={props.hint ?? props.label}
      className={[
        "w-7 h-7 grid place-items-center rounded-[6px]",
        MOTION,
        props.pressed === true ? PRESSED : "hover:bg-[var(--izul-chrome-hover)]",
        "disabled:opacity-35 disabled:pointer-events-none",
        props.className ?? "",
      ].join(" ")}
    >
      <Icon name={props.icon} size={props.size ?? 20} tone={props.tone ?? "plain"} />
    </button>
  );
}

// ---------------------------------------------------------------------------
// Drop-down menus
// ---------------------------------------------------------------------------

export interface MenuItem {
  label: string;
  icon?: IconName;
  tone?: Tone;
  /** Shown right-aligned, e.g. "Ctrl+O"; none when the command has no key. */
  shortcut?: string | undefined;
  onSelect: () => void;
  checked?: boolean;
  disabled?: boolean;
  /** Draws a separator above this item. */
  separator?: boolean;
}

/** Gap between a menu and its button, and between a menu and the window edge. */
const MENU_GAP = 4;

/**
 * Where a menu goes: under its button, or above it when the window has no room
 * below (the zoom menu lives in the bottom bar), and never past either side.
 * Pure, so the placement rules are tested without a layout engine.
 */
export function placeMenu(
  anchor: { left: number; right: number; top: number; bottom: number },
  menu: { width: number; height: number },
  view: { width: number; height: number },
  align: "left" | "right",
): { left: number; top: number; maxHeight: number } {
  const below = view.height - anchor.bottom - MENU_GAP * 2;
  const above = anchor.top - MENU_GAP * 2;
  const up = menu.height > below && above > below;
  const room = Math.max(0, up ? above : below);
  const height = Math.min(menu.height, room);
  const top = up ? anchor.top - MENU_GAP - height : anchor.bottom + MENU_GAP;
  const wanted = align === "right" ? anchor.right - menu.width : anchor.left;
  const left = Math.max(MENU_GAP, Math.min(wanted, view.width - menu.width - MENU_GAP));
  return { left, top, maxHeight: room };
}

/**
 * A button that opens a menu under itself.
 *
 * The menu is portalled to `<body>` and placed with `position: fixed`. It used
 * to be an absolutely placed child of the button, which put it inside the
 * ribbon and the bottom bar — both clip their overflow — so it was cut off, and
 * focusing its first item made the browser scroll those clipped containers to
 * reveal it, shifting the whole ribbon or window content (reported in 7.0.0).
 *
 * Keyboard: Enter/Space/Down open it and focus the first item, Up/Down move,
 * Enter picks, Escape closes and returns focus to the button — the WAI-ARIA
 * menu-button pattern, because that is what a screen reader announces a
 * `aria-haspopup="menu"` button as.
 */
export function MenuButton(props: {
  label: string;
  hint?: string | undefined;
  items: MenuItem[];
  className: string;
  children: ReactNode;
  align?: "left" | "right";
}): JSX.Element {
  const [open, setOpen] = useState(false);
  const [place, setPlace] = useState<{ left: number; top: number; maxHeight: number } | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const align = props.align ?? "left";

  useLayoutEffect(() => {
    if (!open) {
      setPlace(null);
      return;
    }
    const anchor = button.current?.getBoundingClientRect();
    const box = menu.current;
    if (!anchor || !box) return;
    setPlace(
      placeMenu(
        anchor,
        { width: box.offsetWidth, height: box.scrollHeight },
        { width: window.innerWidth, height: window.innerHeight },
        align,
      ),
    );
  }, [open, align]);

  useEffect(() => {
    if (!open) return;
    const inside = (target: EventTarget | null): boolean =>
      target instanceof Node &&
      ((root.current?.contains(target) ?? false) || (menu.current?.contains(target) ?? false));
    const onDown = (e: PointerEvent): void => {
      if (!inside(e.target)) setOpen(false);
    };
    // A fixed menu does not follow its button, so anything that moves the
    // button closes it instead of leaving it floating somewhere else.
    const onScroll = (e: Event): void => {
      if (!inside(e.target)) setOpen(false);
    };
    const onResize = (): void => setOpen(false);
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
    };
  }, [open]);

  useEffect(() => {
    if (!open || place === null) return;
    const first = menu.current?.querySelector<HTMLButtonElement>(
      '[role="menuitem"]:not([disabled]), [role="menuitemcheckbox"]:not([disabled])',
    );
    // preventScroll: focusing must never scroll a container to show the menu.
    first?.focus({ preventScroll: true });
  }, [open, place]);

  function move(e: React.KeyboardEvent): void {
    const items = [
      ...(menu.current?.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not([disabled]), [role="menuitemcheckbox"]:not([disabled])',
      ) ?? []),
    ];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      items[(at + step + items.length) % items.length]?.focus({ preventScroll: true });
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
      button.current?.focus();
    } else if (e.key === "Tab") {
      setOpen(false);
    }
  }

  return (
    <div ref={root} className="relative" onKeyDown={open ? move : undefined}>
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        title={props.hint ?? props.label}
        onClick={() => setOpen(!open)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" && !open) {
            e.preventDefault();
            setOpen(true);
          }
        }}
        className={props.className}
      >
        {props.children}
      </button>
      {open &&
        createPortal(
          <div
            ref={menu}
            id={menuId}
            role="menu"
            aria-label={props.label}
            style={
              place
                ? { left: place.left, top: place.top, maxHeight: place.maxHeight }
                : { left: 0, top: 0, visibility: "hidden" }
            }
            className={[
              "fixed z-[1000] min-w-[200px] py-1 rounded-[8px] overflow-y-auto",
              "bg-[var(--izul-surface)] border border-[var(--izul-border)] shadow-[0_6px_20px_rgba(0,0,0,0.14)]",
            ].join(" ")}
          >
            {props.items.map((item) => (
              <div key={item.label}>
                {item.separator === true && <div className="my-1 h-px bg-[var(--izul-border)]" />}
                <button
                  type="button"
                  role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
                  aria-checked={item.checked}
                  disabled={item.disabled ?? false}
                  onClick={() => {
                    setOpen(false);
                    item.onSelect();
                  }}
                  className="w-full h-8 px-3 flex items-center gap-2.5 text-left text-[13px] hover:bg-[var(--izul-surface-raised)] focus:bg-[var(--izul-surface-raised)] focus:outline-none disabled:opacity-40"
                >
                  <span className="w-5 grid place-items-center">
                    {item.icon ? (
                      <Icon name={item.icon} size={20} tone={item.tone ?? "neutral"} />
                    ) : item.checked === true ? (
                      "✓"
                    ) : null}
                  </span>
                  <span className="flex-1">{item.label}</span>
                  {item.shortcut && (
                    <span className="text-[12px] text-[var(--izul-text-dim)]">{item.shortcut}</span>
                  )}
                </button>
              </div>
            ))}
          </div>,
          document.body,
        )}
    </div>
  );
}
