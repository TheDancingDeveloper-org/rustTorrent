import { MouseEvent, ReactNode } from "react";

// Literal lookup so Tailwind can see every class name at build time.
const colorClassNames = {
  primary: "text-primary",
  secondary: "text-secondary",
  success: "text-success",
  warning: "text-warning",
  error: "text-error",
} as const;

export type IconButtonColor = keyof typeof colorClassNames;

export const IconButton: React.FC<{
  onClick: () => void;
  label?: string;
  disabled?: boolean;
  className?: string;
  color?: IconButtonColor;
  children: ReactNode;
  href?: string;
}> = ({ onClick, label, disabled, color, children, className, href }) => {
  const classes = [
    "p-1.5 flex items-center justify-center cursor-pointer",
    "disabled:cursor-not-allowed disabled:opacity-50",
    "focus-ring touch-target",
    colorClassNames[color ?? "primary"],
    className ?? "",
  ].join(" ");

  const onActivate = (e: MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
    if (disabled) {
      return;
    }
    onClick();
  };

  if (href) {
    return (
      <a
        className={classes}
        href={href}
        onClick={onActivate}
        aria-label={label}
        aria-disabled={disabled || undefined}
        title={label}
      >
        {children}
      </a>
    );
  }

  return (
    <button
      type="button"
      className={classes}
      disabled={disabled}
      onClick={onActivate}
      aria-label={label}
      title={label}
    >
      {children}
    </button>
  );
};
