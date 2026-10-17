import { useEffect } from "react";
import { BsX } from "react-icons/bs";
import {
  Toast as ToastItem,
  ToastTone,
  useToastStore,
} from "../stores/toastStore";

// Errors stay longer so they can be read and acted on
const DISMISS_MS: Record<ToastTone, number> = {
  info: 4000,
  success: 4000,
  error: 8000,
};

const toneBorder: Record<ToastTone, string> = {
  info: "border-divider",
  success: "border-success",
  error: "border-error",
};

const ToastView = ({ toast }: { toast: ToastItem }) => {
  const dismissToast = useToastStore((state) => state.dismissToast);

  useEffect(() => {
    const timer = setTimeout(
      () => dismissToast(toast.id),
      DISMISS_MS[toast.tone],
    );
    return () => clearTimeout(timer);
  }, [toast.id, toast.tone, dismissToast]);

  return (
    <div
      className={`flex items-start gap-2 rounded-md border bg-surface shadow-lg px-3 py-2 text-sm text-primary ${toneBorder[toast.tone]}`}
    >
      <p className="flex-1 min-w-0 break-words">{toast.text}</p>
      <button
        type="button"
        aria-label="Dismiss notification"
        title="Dismiss"
        onClick={() => dismissToast(toast.id)}
        className="p-0.5 text-secondary hover:text-primary cursor-pointer rounded focus-ring touch-target"
      >
        <BsX className="w-4 h-4" />
      </button>
    </div>
  );
};

/** Stack of transient notifications, bottom-right; the page keeps working under it. */
export const ToastViewport = () => {
  const toasts = useToastStore((state) => state.toasts);

  return (
    <div
      role="status"
      aria-live="polite"
      className="fixed bottom-4 right-4 z-[60] flex flex-col gap-2 w-80 max-w-[calc(100vw-2rem)] pointer-events-none"
    >
      {toasts.map((toast) => (
        <div key={toast.id} className="pointer-events-auto">
          <ToastView toast={toast} />
        </div>
      ))}
    </div>
  );
};
