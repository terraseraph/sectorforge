import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode } from "react";

export const cx = (...names: (string | false | null | undefined)[]) => names.filter(Boolean).join(" ");

export type Tone = "accent" | "green" | "amber" | "red" | "dim";

export function Panel({ title, label, actions, children }: { title?: string; label?: string; actions?: ReactNode; children: ReactNode }) {
  return (
    <section className="panel" aria-label={label ?? title}>
      {(title || actions) && (
        <div className="panel-head">
          {title && <h3>{title}</h3>}
          {actions}
        </div>
      )}
      {children}
    </section>
  );
}

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "default" | "primary" | "link";
  small?: boolean;
}

export function Button({ variant = "default", small, className, ...rest }: ButtonProps) {
  return <button type="button" className={cx(variant === "link" ? "lnk" : "btn", variant === "primary" && "primary", small && "small", className)} {...rest} />;
}

export function Badge({ tone, children }: { tone: Tone; children: ReactNode }) {
  return <span className={cx("badge", `tone-${tone}`)}>{children}</span>;
}

export function Notice({ tone = "info", children }: { tone?: "info" | "warn" | "danger"; children: ReactNode }) {
  return <div className={cx("notice", tone !== "info" && tone)}>{children}</div>;
}

export function ProgressBar({ value, label }: { value: number; label: string }) {
  const v = Math.max(0, Math.min(100, value));
  return (
    <div className="bar" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(v)}>
      <div style={{ width: `${v}%` }} />
    </div>
  );
}

/** Label, control, and an optional button on one row. */
export function Field({ id, label, action, children }: { id: string; label: string; action?: ReactNode; children: ReactNode }) {
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      {children}
      {action ?? <span />}
    </div>
  );
}

export function Hint({ children, flush }: { children: ReactNode; flush?: boolean }) {
  return <p className={cx("hint", flush && "flush")}>{children}</p>;
}

export function TextInput(props: InputHTMLAttributes<HTMLInputElement>) {
  return <input type="text" spellCheck={false} autoComplete="off" {...props} />;
}

export function Check({ checked, onChange, children }: { checked: boolean; onChange: (on: boolean) => void; children: ReactNode }) {
  return (
    <label className="check">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      {children}
    </label>
  );
}
