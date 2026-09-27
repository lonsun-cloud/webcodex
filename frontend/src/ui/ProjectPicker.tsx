import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export type ProjectPickerOption = { value: string; label: string; detail?: string };

type Props = {
  label: string;
  allLabel?: string;
  emptyLabel: string;
  searchLabel: string;
  options: ProjectPickerOption[];
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  className?: string;
  kind?: "project" | "runner";
};

export function ProjectGlyph({ all = false }: { all?: boolean }) {
  return all
    ? <svg aria-hidden="true" viewBox="0 0 20 20" fill="none"><rect x="2.5" y="2.5" width="6" height="6" rx="1.4"/><rect x="11.5" y="2.5" width="6" height="6" rx="1.4"/><rect x="2.5" y="11.5" width="6" height="6" rx="1.4"/><rect x="11.5" y="11.5" width="6" height="6" rx="1.4"/></svg>
    : <svg aria-hidden="true" viewBox="0 0 20 20" fill="none"><path d="M2.5 6.5a2 2 0 0 1 2-2h3.7l1.8 1.8h5.5a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-11a2 2 0 0 1-2-2v-8.8Z"/></svg>;
}

function PickerGlyph({ kind, all }: { kind: "project" | "runner"; all?: boolean }) {
  if (kind === "project") return <ProjectGlyph all={all} />;
  return <svg aria-hidden="true" viewBox="0 0 20 20" fill="none"><rect x="3" y="3" width="14" height="5.5" rx="1.5"/><rect x="3" y="11.5" width="14" height="5.5" rx="1.5"/><path d="M6.5 5.75h.01M6.5 14.25h.01"/></svg>;
}

/** One resource menu used by the Runtime and Desktop work surfaces. */
export function ScopePicker({ label, allLabel, emptyLabel, searchLabel, options, value, onChange, disabled = false, className = "", kind = "project" }: Props) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [position, setPosition] = useState({ top: 0, left: 0, width: 260, maxHeight: 320 });
  const selected = options.find((option) => option.value === value);
  const visible = options.filter((option) => `${option.label} ${option.detail || ""}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));

  useEffect(() => {
    if (disabled) { setOpen(false); setQuery(""); }
  }, [disabled]);

  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const rect = trigger.current?.getBoundingClientRect();
      if (!rect) return;
      const width = Math.min(Math.max(rect.width, 248), window.innerWidth - 24);
      const left = Math.max(12, Math.min(rect.left, window.innerWidth - width - 12));
      const below = window.innerHeight - rect.bottom - 16;
      const above = rect.top - 16;
      const useAbove = below < 220 && above > below;
      const maxHeight = Math.max(120, Math.min(340, useAbove ? above - 8 : below - 8));
      setPosition({ top: useAbove ? Math.max(8, rect.top - maxHeight - 8) : rect.bottom + 8, left, width, maxHeight });
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => { window.removeEventListener("resize", place); window.removeEventListener("scroll", place, true); };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: PointerEvent) => {
      if (!trigger.current?.contains(event.target as Node) && !panel.current?.contains(event.target as Node)) { setOpen(false); setQuery(""); }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.stopPropagation(); setOpen(false); setQuery(""); trigger.current?.focus(); }
    };
    document.addEventListener("pointerdown", closeOutside);
    document.addEventListener("keydown", closeOnEscape, true);
    if (options.length > 5) search.current?.focus();
    else {
      const items = Array.from(panel.current?.querySelectorAll<HTMLButtonElement>(".project-picker-option") || []);
      (items.find((item) => item.dataset.value === value) || items[0])?.focus();
    }
    return () => { document.removeEventListener("pointerdown", closeOutside); document.removeEventListener("keydown", closeOnEscape, true); };
  }, [open, options.length, value]);

  const choose = (next: string) => {
    if (disabled) return;
    onChange(next);
    setOpen(false);
    setQuery("");
    trigger.current?.focus();
  };

  return <div className={`project-picker ${className}`}>
    <button ref={trigger} type="button" className="project-picker-trigger" aria-label={`${label}: ${selected?.label || allLabel || label}`} aria-haspopup="dialog" aria-expanded={open && !disabled} aria-controls={open && !disabled ? id : undefined} disabled={disabled} onClick={() => { if (open) setQuery(""); setOpen((current) => !current); }}>
      <span className="project-picker-trigger-icon"><PickerGlyph kind={kind} all={!value} /></span>
      <span className="project-picker-value">{selected?.label || allLabel || label}</span>
      <svg className="project-picker-chevron" aria-hidden="true" viewBox="0 0 20 20" fill="none"><path d="m5 7.5 5 5 5-5"/></svg>
    </button>
    {open && !disabled && createPortal(<div ref={panel} id={id} className="project-picker-popover" role="dialog" aria-label={label} style={position} onKeyDown={(event) => {
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      event.preventDefault();
      const items = Array.from(panel.current?.querySelectorAll<HTMLButtonElement>(".project-picker-option") || []);
      const index = items.findIndex((item) => item === document.activeElement);
      const next = event.key === "ArrowDown" ? (index + 1) % items.length : (index - 1 + items.length) % items.length;
      items[next]?.focus();
    }}>
      <div className="project-picker-heading"><strong>{label}</strong><span>{options.length}</span></div>
      {options.length > 5 && <label className="project-picker-search"><svg aria-hidden="true" viewBox="0 0 20 20" fill="none"><circle cx="8.5" cy="8.5" r="5.5"/><path d="m13 13 4 4"/></svg><input ref={search} aria-label={searchLabel} placeholder={searchLabel} value={query} onChange={(event) => setQuery(event.target.value)} /></label>}
      <div className="project-picker-options">
        {allLabel && !query && <button className="project-picker-option" data-value="" type="button" aria-pressed={!value} onClick={() => choose("")}><span className="project-picker-option-icon"><PickerGlyph kind={kind} all /></span><span>{allLabel}</span><span className="project-picker-check" aria-hidden="true">✓</span></button>}
        {visible.map((option) => <button className="project-picker-option" data-value={option.value} key={option.value} type="button" aria-pressed={value === option.value} onClick={() => choose(option.value)}><span className="project-picker-option-icon"><PickerGlyph kind={kind} /></span><span className="project-picker-option-copy"><strong>{option.label}</strong>{option.detail && <small>{option.detail}</small>}</span><span className="project-picker-check" aria-hidden="true">✓</span></button>)}
        {!visible.length && <p className="project-picker-empty">{emptyLabel}</p>}
      </div>
    </div>, document.body)}
  </div>;
}

export const ProjectPicker = ScopePicker;
