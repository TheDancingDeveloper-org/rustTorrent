import React, { createContext, useContext, useEffect, useId, useRef } from "react";

// Ids shared by a TabList, its TabButtons and the matching TabPanels.
// `linked` is set only when the caller renders TabPanels with the same
// prefix, so tabs never point aria-controls at a panel that doesn't exist.
interface TabIds {
  prefix: string;
  linked: boolean;
}

const TabIdsContext = createContext<TabIds | null>(null);

export const tabElementId = (prefix: string, id: string) => `${prefix}-tab-${id}`;
export const tabPanelElementId = (prefix: string, id: string) =>
  `${prefix}-panel-${id}`;

export interface TabButtonProps {
  id: string;
  label: string;
  active: boolean;
  onClick: () => void;
}

export const TabButton: React.FC<TabButtonProps> = ({
  id,
  label,
  active,
  onClick,
}) => {
  const ids = useContext(TabIdsContext);
  const ref = useRef<HTMLButtonElement>(null);

  // Keep the active tab visible when the list scrolls horizontally
  useEffect(() => {
    if (active) {
      ref.current?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    }
  }, [active]);

  return (
    <button
      ref={ref}
      type="button"
      role="tab"
      id={ids ? tabElementId(ids.prefix, id) : undefined}
      aria-selected={active}
      aria-controls={
        ids?.linked ? tabPanelElementId(ids.prefix, id) : undefined
      }
      tabIndex={active ? 0 : -1}
      onClick={onClick}
      className={`shrink-0 snap-start whitespace-nowrap px-3 py-1.5 font-medium transition-colors cursor-pointer focus-ring ${
        active
          ? "border-b-2 border-primary text-primary -mb-px"
          : "text-tertiary hover:text-secondary"
      }`}
    >
      {label}
    </button>
  );
};

export interface TabListProps {
  children: React.ReactNode;
  className?: string;
  label?: string;
  // Pass the same prefix to the TabPanels to link tabs and panels
  idPrefix?: string;
}

const KEY_STEPS: Record<string, (index: number, count: number) => number> = {
  ArrowRight: (i, n) => (i + 1) % n,
  ArrowLeft: (i, n) => (i - 1 + n) % n,
  Home: () => 0,
  End: (_, n) => n - 1,
};

export const TabList: React.FC<TabListProps> = ({
  children,
  className,
  label,
  idPrefix,
}) => {
  const generated = useId();
  const ids: TabIds = {
    prefix: idPrefix ?? generated,
    linked: idPrefix !== undefined,
  };

  // Arrow keys move between tabs and select them (automatic activation)
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const step = KEY_STEPS[e.key];
    if (!step) return;
    const tabs = Array.from(
      e.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]'),
    );
    const current = tabs.indexOf(document.activeElement as HTMLButtonElement);
    if (current === -1 || tabs.length === 0) return;
    e.preventDefault();
    const next = tabs[step(current, tabs.length)];
    next.focus();
    next.click();
  };

  return (
    <TabIdsContext.Provider value={ids}>
      <div
        role="tablist"
        aria-label={label}
        onKeyDown={onKeyDown}
        className={`flex border-b border-divider overflow-x-auto snap-x [scrollbar-width:none] [&::-webkit-scrollbar]:hidden ${className ?? ""}`}
      >
        {children}
      </div>
    </TabIdsContext.Provider>
  );
};

export interface TabPanelProps {
  id: string;
  idPrefix: string;
  active: boolean;
  children: React.ReactNode;
}

export const TabPanel: React.FC<TabPanelProps> = ({
  id,
  idPrefix,
  active,
  children,
}) => (
  <div
    role="tabpanel"
    id={tabPanelElementId(idPrefix, id)}
    aria-labelledby={tabElementId(idPrefix, id)}
    hidden={!active}
  >
    {children}
  </div>
);
