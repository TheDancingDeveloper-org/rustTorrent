// Fill colour per variant. The label is drawn twice: once in text colour over
// the track, and once in white clipped to the filled part, so it stays legible
// on both the track and the fill.
const fillClassNames = {
  warn: "bg-warning-bg",
  info: "bg-primary-bg",
  success: "bg-success-bg",
  error: "bg-error-bg",
};

export const ProgressBar: React.FC<{
  now: number;
  label?: string | null;
  variant?: "warn" | "info" | "success" | "error";
  classNames?: string;
}> = ({ now, variant, label, classNames }) => {
  const progressLabel = label ?? `${now.toFixed(2)}%`;
  const clamped = Math.min(Math.max(now, 0), 100);

  const fillClassName =
    now < 1
      ? "bg-transparent"
      : (fillClassNames[variant ?? "info"] ?? fillClassNames.info);

  const labelClassName =
    "absolute inset-0 flex items-center justify-center text-sm font-medium leading-none whitespace-nowrap";

  return (
    <div
      role="progressbar"
      aria-valuenow={clamped}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuetext={progressLabel}
      className={`relative h-5 w-full overflow-hidden rounded-full bg-divider mb-1 ${classNames ?? ""}`}
    >
      <div
        className={`absolute inset-y-0 left-0 transition-all ${fillClassName}`}
        style={{ width: `${clamped}%` }}
      />
      <span className={`${labelClassName} text-text`}>{progressLabel}</span>
      <span
        aria-hidden="true"
        className={`${labelClassName} text-on-primary`}
        style={{ clipPath: `inset(0 ${100 - clamped}% 0 0)` }}
      >
        {progressLabel}
      </span>
    </div>
  );
};
