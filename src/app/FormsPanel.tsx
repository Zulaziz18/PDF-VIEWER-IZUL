/**
 * The "Formulir" side panel (Phase 7): every field of the document's form,
 * grouped by page, each with the control its kind needs.
 *
 * A panel rather than typing into the page: the page is a tile raster, and a
 * text box drawn over it would have to match the widget's font, size, comb
 * and alignment to not jump when the tile under it is rendered again. PDFium
 * builds the widget's appearance from the value; the page shows that, and
 * this panel is where the value is typed.
 *
 * Text commits on blur or Enter, not per keystroke: each commit is one undo
 * step and one repaint of the page, and "Ctrl+Z undid one letter" is not what
 * anyone wants from a name field.
 */

import { useEffect, useState, type JSX } from "react";
import { Icon } from "@/design/Icon";
import { fill } from "./fileActions";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { viewport } from "./viewportHandle";
import {
  byPage,
  checkForForm,
  controlOf,
  formFields,
  labelOf,
  sameValue,
  setFormValue,
  useFormPresence,
  type FormField,
  type FormValue,
} from "./forms";

const INPUT =
  "w-full min-w-0 rounded-[6px] bg-[var(--izul-canvas)] border border-[var(--izul-border)] px-2 py-1 text-[13px] disabled:opacity-60";

function goTo(field: FormField): void {
  const first = field.widgets.reduce((a, b) => (b[0] < a[0] ? b : a));
  const store = useDocument.getState();
  // Widgets are on the file's pages; after pages have been moved the page may
  // be elsewhere on screen.
  const shown = store.displayOfOwn(first[0]);
  if (shown === null) return;
  store.setPage(shown);
  viewport()?.goToPage(shown, first[1].top + 24);
}

function TextField(props: { field: FormField; multiline: boolean }): JSX.Element {
  const value = "Text" in props.field.value ? props.field.value.Text : "";
  const [draft, setDraft] = useState(value);
  // The value changed underneath — an undo, another commit — and the box
  // follows it.
  useEffect(() => setDraft(value), [value]);
  const commit = (): void => {
    const next: FormValue = { Text: draft };
    if (!sameValue(next, props.field.value)) void setFormValue(props.field.name, next);
  };
  const common = {
    value: draft,
    disabled: props.field.read_only,
    "aria-label": labelOf(props.field.name),
    onFocus: () => goTo(props.field),
    onBlur: commit,
    className: INPUT,
  };
  return props.multiline ? (
    <textarea
      {...common}
      rows={3}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) commit();
        if (e.key === "Escape") setDraft(value);
      }}
    />
  ) : (
    <input
      {...common}
      type="text"
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") setDraft(value);
      }}
    />
  );
}

function Control(props: { field: FormField }): JSX.Element {
  const { field } = props;
  const control = controlOf(field.kind);
  const label = labelOf(field.name);
  switch (control) {
    case "text":
    case "multiline":
      return <TextField field={field} multiline={control === "multiline"} />;
    case "checkbox": {
      const checked = "Checked" in field.value && field.value.Checked;
      return (
        <label className="flex items-center gap-2 text-[13px]">
          <input
            type="checkbox"
            checked={checked}
            disabled={field.read_only}
            onChange={(e) => {
              goTo(field);
              void setFormValue(field.name, { Checked: e.target.checked });
            }}
          />
          {checked ? t("forms.checked") : t("forms.unchecked")}
        </label>
      );
    }
    case "radio": {
      const current = "Radio" in field.value ? field.value.Radio : "";
      const choices = [...new Set(field.exports)];
      return (
        <div role="radiogroup" aria-label={label} className="flex flex-col gap-1">
          {choices.map((option) => (
            <label key={option} className="flex items-center gap-2 text-[13px]">
              <input
                type="radio"
                name={`form-${field.name}`}
                checked={current === option}
                disabled={field.read_only}
                onChange={() => {
                  goTo(field);
                  void setFormValue(field.name, { Radio: option });
                }}
              />
              {option}
            </label>
          ))}
          {current !== "" && !field.read_only && (
            <button
              type="button"
              className="self-start text-[12px] text-[var(--izul-accent)] hover:underline"
              onClick={() => void setFormValue(field.name, { Radio: "" })}
            >
              {t("forms.clearChoice")}
            </button>
          )}
        </div>
      );
    }
    case "select":
    case "list": {
      const selected = "Choice" in field.value ? field.value.Choice : [];
      const multiple = typeof field.kind === "object" && "ListBox" in field.kind && field.kind.ListBox.multiple;
      return (
        <select
          aria-label={label}
          className={INPUT}
          disabled={field.read_only}
          multiple={multiple}
          size={multiple ? Math.min(5, field.options.length) : undefined}
          value={multiple ? selected.map(String) : String(selected[0] ?? "")}
          onFocus={() => goTo(field)}
          onChange={(e) => {
            const picked = [...e.target.selectedOptions].map((o) => Number(o.value)).filter((n) => n >= 0);
            void setFormValue(field.name, { Choice: picked });
          }}
        >
          {!multiple && <option value="-1">{t("forms.noChoice")}</option>}
          {field.options.map((option, index) => (
            <option key={`${index}-${option}`} value={String(index)}>
              {option}
            </option>
          ))}
        </select>
      );
    }
  }
}

export function FormsPanel(): JSX.Element {
  const doc = useDocument((s) => s.doc);
  const revision = useDocument((s) => s.formRevision);
  const [fields, setFields] = useState<FormField[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (doc === null) return;
    let live = true;
    formFields(doc)
      .then((list) => {
        if (!live) return;
        setFields(list);
        setError(null);
        useFormPresence.getState().known(doc, list.length);
      })
      .catch((e: unknown) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, [doc, revision]);

  if (error !== null) {
    return (
      <p role="alert" className="p-3 text-[12px] text-[var(--izul-danger)]">
        {t("forms.loadFailed")} {error}
      </p>
    );
  }
  if (fields === null) return <p className="p-3 text-[13px] text-[var(--izul-text-dim)]">{t("forms.loading")}</p>;
  if (fields.length === 0) return <p className="p-3 text-[13px] text-[var(--izul-text-dim)]">{t("forms.empty")}</p>;

  return (
    <div className="p-2 flex flex-col gap-3">
      <p className="px-1 text-[12px] leading-snug text-[var(--izul-text-dim)]">{t("forms.hint")}</p>
      {byPage(fields).map(([page, list]) => (
        <section key={page} aria-label={fill(t("forms.page"), { n: String(page + 1) })}>
          <h3 className="px-1 pb-1 text-[11px] font-semibold uppercase tracking-wide text-[var(--izul-text-dim)]">
            {fill(t("forms.page"), { n: String(page + 1) })}
          </h3>
          <ul className="flex flex-col gap-2">
            {list.map((field) => (
              <li key={field.name} className="px-1 flex flex-col gap-1">
                <button
                  type="button"
                  onClick={() => goTo(field)}
                  title={field.name}
                  className="self-start max-w-full truncate text-left text-[12px] font-medium hover:text-[var(--izul-accent)]"
                >
                  {labelOf(field.name)}
                  {field.read_only && <span className="ml-1 font-normal text-[var(--izul-text-dim)]">{t("forms.readOnly")}</span>}
                </button>
                <Control field={field} />
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

/**
 * The strip above the page that says the document has a form, and where to
 * fill it — the panel is otherwise one icon among several on the rail, and a
 * form nobody knows can be filled is a form printed and filled by hand.
 */
export function FormBanner(): JSX.Element | null {
  const doc = useDocument((s) => s.doc);
  const open = useDocument((s) => s.sidebarOpen && s.sidebarTab === "forms");
  const count = useFormPresence((s) => (doc === null ? undefined : s.count.get(doc)));
  const dismissed = useFormPresence((s) => doc !== null && s.dismissed.has(doc));
  useEffect(() => {
    if (doc !== null) void checkForForm(doc);
  }, [doc]);
  if (doc === null || open || dismissed || count === undefined || count === 0) return null;
  return (
    <div
      role="status"
      className="shrink-0 flex items-center gap-3 px-3 py-2 bg-[var(--izul-accent-soft)] border-b border-[var(--izul-border)] text-[13px]"
    >
      <Icon name="formFill" size={20} tone="teal" />
      <p className="flex-1 min-w-0">{fill(t("forms.banner"), { n: String(count) })}</p>
      <button
        type="button"
        className="h-7 px-3 rounded-[6px] text-[12px] border border-[var(--izul-accent)] text-[var(--izul-accent)] font-medium bg-[var(--izul-surface)] hover:bg-[var(--izul-surface-raised)]"
        onClick={() => useDocument.getState().setSidebarTab("forms")}
      >
        {t("forms.fill")}
      </button>
      <button
        type="button"
        className="h-7 px-3 rounded-[6px] text-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] hover:bg-[var(--izul-surface-raised)]"
        onClick={() => useFormPresence.getState().dismiss(doc)}
      >
        {t("forms.later")}
      </button>
    </div>
  );
}
