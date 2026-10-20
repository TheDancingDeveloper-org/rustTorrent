import { BsX } from "react-icons/bs";
import { ErrorDetails } from "../api-types";
import { ErrorWithLabel } from "../rtbit-web";
import { Button } from "./buttons/Button";

const AlertDanger: React.FC<{
  title: string;
  children: React.ReactNode;
  onClose?: () => void;
}> = ({ title, children, onClose }) => {
  return (
    <div className="bg-error/10 p-3 rounded-md mb-3">
      <div className="flex justify-between mb-2">
        <h2 className="text-lg font-semibold text-error">{title}</h2>
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            aria-label="Dismiss"
            className="text-error hover:text-error/80 transition-colors cursor-pointer focus-ring rounded"
          >
            <BsX />
          </button>
        )}
      </div>
      {children}
    </div>
  );
};

export const ErrorComponent = (props: {
  error: ErrorWithLabel | null;
  remove?: () => void;
}) => {
  const { error, remove } = props;

  if (error == null) {
    return null;
  }

  return (
    <AlertDanger onClose={remove} title={error.text}>
      {error.details?.statusText && (
        <div className="pb-2 text-md">{error.details?.statusText}</div>
      )}
      <div className="whitespace-pre-wrap text-secondary">
        {error.details?.text}
      </div>
    </AlertDanger>
  );
};

/** Full-panel state shown when the torrent refresh cannot reach the engine. */
export const EngineUnreachable = (props: {
  apiBase: string;
  details: ErrorDetails | null;
  onRetry: () => void;
}) => {
  const { apiBase, details, onRetry } = props;

  return (
    <div className="h-full flex items-center justify-center p-4">
      <div className="w-full max-w-md bg-error/10 p-4 rounded-md flex flex-col gap-3">
        <h2 className="text-lg font-semibold text-error">
          Can&apos;t reach the rustTorrent engine
        </h2>
        <p className="text-sm text-secondary break-all">
          API: <code>{apiBase}</code>
        </p>
        <p role="status" aria-live="polite" className="text-sm text-secondary">
          Retrying automatically…
        </p>
        <div>
          <Button onClick={onRetry} variant="primary">
            Retry
          </Button>
        </div>
        <details className="text-sm text-secondary">
          <summary className="cursor-pointer">Technical details</summary>
          <div className="pt-2 whitespace-pre-wrap break-all">
            {details?.statusText && <div>{details.statusText}</div>}
            <div>{details?.text || "network error"}</div>
          </div>
        </details>
      </div>
    </div>
  );
};
