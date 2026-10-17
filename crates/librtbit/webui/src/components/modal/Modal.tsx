// Modal.tsx
import React, { useId, type ReactNode } from "react";
import RestartModal from "@restart/ui/Modal";
import { BsX } from "react-icons/bs";

interface ModalProps {
  isOpen: boolean;
  onClose?: () => void;
  title: string;
  children: ReactNode;
  className?: string;
}

const ModalHeader: React.FC<{
  id: string;
  onClose?: () => void;
  title: string;
}> = ({ id, onClose, title }) => {
  return (
    <div className="flex p-3 justify-between items-center border-b border-divider gap-2">
      <h2 id={id} className="text-xl font-semibold truncate min-w-0">
        {title}
      </h2>
      {onClose && (
        <button
          className="flex-shrink-0 p-1 -m-1 text-secondary hover:text-text transition-colors cursor-pointer"
          onClick={onClose}
          aria-label="Close modal"
        >
          <BsX className="w-6 h-6" />
        </button>
      )}
    </div>
  );
};

export const Modal: React.FC<ModalProps> = ({
  isOpen,
  onClose,
  title,
  children,
  className,
}) => {
  // The dialog is named by its title, so screen readers announce what it is
  const titleId = useId();
  const renderBackdrop = () => {
    return (
      <div className="fixed inset-0 bg-black/50 z-300 dark:bg-black/70"></div>
    );
  };
  return (
    <RestartModal
      show={isOpen}
      aria-labelledby={titleId}
      onHide={onClose}
      renderBackdrop={renderBackdrop}
      className="fixed z-301 top-0 left-0 w-full h-full block overflow-x-hidden overflow-y-auto"
    >
      <div
        className={`bg-surface-raised shadow-lg my-4 mx-4 sm:my-8 sm:mx-auto sm:max-w-2xl rounded ${className}`}
      >
        <ModalHeader id={titleId} onClose={onClose} title={title} />
        {children}
      </div>
    </RestartModal>
  );
};
