interface StatProps {
  label: string;
  value: string;
  mono?: boolean;
}

export function Stat({ label, value, mono }: StatProps) {
  return (
    <div className="stat">
      <span className="stat-label">{label}</span>
      <span className={`stat-value${mono ? " mono" : ""}`}>{value}</span>
    </div>
  );
}

interface ProgressProps {
  /** 0..=1, or null when unknown — renders an honest indeterminate state. */
  value: number | null;
  label: string;
  detail?: string;
}

export function ProgressBar({ value, label, detail }: ProgressProps) {
  const pct = value != null ? Math.round(value * 100) : null;
  return (
    <div className="progress-block">
      <div className="progress-row">
        <span className="progress-label">{label}</span>
        <span className="progress-pct mono">{pct != null ? `${pct}%` : "—"}</span>
      </div>
      <div
        className={`progress-track${pct == null ? " indeterminate" : ""}`}
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct ?? undefined}
        aria-label={label}
      >
        {pct != null ? <div className="progress-fill" style={{ width: `${pct}%` }} /> : null}
      </div>
      {detail ? <div className="progress-detail">{detail}</div> : null}
    </div>
  );
}
