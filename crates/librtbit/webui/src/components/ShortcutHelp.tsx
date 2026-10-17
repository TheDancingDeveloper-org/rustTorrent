import React, { useEffect } from "react";
import { Modal } from "./modal/Modal";
import { ModalBody } from "./modal/ModalBody";
import { useUIStore } from "../stores/uiStore";
import { SHORTCUTS } from "../helper/shortcutList";

// "?" overlay listing the keyboard shortcuts. It is a dialog, so the global
// shortcut handler stands aside while it is open; it closes itself on Esc or ?.
export const ShortcutHelp: React.FC = () => {
  const open = useUIStore((state) => state.shortcutHelpOpen);
  const setOpen = useUIStore((state) => state.setShortcutHelpOpen);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" || e.key === "?") {
        e.preventDefault();
        setOpen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, setOpen]);

  if (!open) return null;

  return (
    <Modal
      isOpen={open}
      onClose={() => setOpen(false)}
      title="Keyboard shortcuts"
      className="sm:max-w-lg"
    >
      <ModalBody>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
          {SHORTCUTS.map((entry) => (
            <React.Fragment key={entry.keys}>
              <dt className="font-mono whitespace-nowrap text-primary">
                {entry.keys}
              </dt>
              <dd className="text-secondary">{entry.action}</dd>
            </React.Fragment>
          ))}
        </dl>
      </ModalBody>
    </Modal>
  );
};
