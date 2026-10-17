import React from "react";
import { useUIStore } from "../stores/uiStore";

// Polite live region for state changes (see hooks/useAnnouncements). It is
// visually hidden. The zero-width suffix flips on every announcement so a
// repeated message still changes the text and is read out again.
export const LiveAnnouncer: React.FC = () => {
  const { text, seq } = useUIStore((state) => state.announcement);
  return (
    <div
      role="status"
      aria-live="polite"
      aria-atomic="true"
      className="sr-only"
    >
      {text}
      {seq % 2 === 1 ? "\u200B" : ""}
    </div>
  );
};
