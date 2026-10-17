import React, { useState, type ReactNode } from "react";
import { Modal } from "./Modal";
import { ModalBody } from "./ModalBody";
import { ModalFooter } from "./ModalFooter";
import { Button } from "../buttons/Button";
import { ErrorComponent } from "../ErrorComponent";
import { ErrorWithLabel } from "../../rtbit-web";
import { TabButton, TabList } from "../Tabs";

export interface ConfigTab {
  id: string;
  label: string;
  // Extra words the settings search matches, e.g. the names of fields on the tab
  keywords?: string;
  content: ReactNode;
}

export interface TabbedConfigModalProps {
  isOpen: boolean;
  onClose?: () => void;
  title: string;
  tabs: ConfigTab[];
  initialTab?: string;
  onSave: () => void | Promise<void>;
  onReset?: () => void;
  saveButtonText?: string;
  showResetButton?: boolean;
  isSaving?: boolean;
  error?: ErrorWithLabel | null;
}

export const TabbedConfigModal: React.FC<TabbedConfigModalProps> = ({
  isOpen,
  onClose,
  title,
  tabs,
  initialTab,
  onSave,
  onReset,
  saveButtonText = "OK",
  showResetButton = false,
  isSaving = false,
  error,
}) => {
  const [currentTab, setCurrentTab] = useState(initialTab || tabs[0]?.id);
  const [query, setQuery] = useState("");

  const needle = query.trim().toLowerCase();
  const visibleTabs = needle
    ? tabs.filter((tab) =>
        `${tab.label} ${tab.keywords ?? ""}`.toLowerCase().includes(needle),
      )
    : tabs;
  const activeTab = visibleTabs.some((tab) => tab.id === currentTab)
    ? currentTab
    : visibleTabs[0]?.id;

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title={title}
      className="sm:max-w-4xl"
    >
      <ModalBody>
        <ErrorComponent error={error ?? null} />
        {tabs.length > 1 && (
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search settings"
            aria-label="Search settings"
            className="w-full mb-3 px-2 py-1.5 rounded-md border border-divider bg-surface text-text"
          />
        )}
        {tabs.length > 1 && (
          <TabList className="mb-4">
            {visibleTabs.map((tab) => (
              <TabButton
                key={tab.id}
                id={tab.id}
                label={tab.label}
                active={tab.id === activeTab}
                onClick={() => setCurrentTab(tab.id)}
              />
            ))}
          </TabList>
        )}
        {needle && visibleTabs.length === 0 && (
          <p className="text-secondary text-sm">No settings match "{query}".</p>
        )}
        {tabs.map((tab) => (
          <div key={tab.id} className={tab.id === activeTab ? "" : "hidden"}>
            {tab.content}
          </div>
        ))}
      </ModalBody>
      <ModalFooter>
        {onClose && (
          <Button variant="cancel" onClick={onClose}>
            Cancel
          </Button>
        )}
        {showResetButton && onReset && (
          <Button variant="secondary" onClick={onReset}>
            Reset to defaults
          </Button>
        )}
        <Button variant="primary" onClick={onSave} disabled={isSaving}>
          {saveButtonText}
        </Button>
      </ModalFooter>
    </Modal>
  );
};
