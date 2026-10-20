import React, {
  useEffect,
  useId,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Modal } from "./Modal";
import { ModalBody } from "./ModalBody";
import { ModalFooter } from "./ModalFooter";
import { Button } from "../buttons/Button";
import { ErrorComponent } from "../ErrorComponent";
import { ErrorWithLabel } from "../../rtbit-web";
import { TabButton, TabList, TabPanel } from "../Tabs";
import { RestartRequiredBadge } from "../config/SettingNotes";
import type { SettingDefinition } from "../config/settingsRegistry";
import { matchSettings } from "../../helper/settingsSearch";

export interface ConfigTab {
  id: string;
  label: string;
  content: ReactNode;
}

export interface TabbedConfigModalProps {
  isOpen: boolean;
  onClose?: () => void;
  title: string;
  tabs: ConfigTab[];
  // Individual settings to search by label; each one names the tab it lives on
  settings?: readonly SettingDefinition[];
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
  settings = [],
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
  // Set by a search result; the field is focused once its tab is showing
  const [focusSettingId, setFocusSettingId] = useState<string | null>(null);
  const tabsRef = useRef<HTMLDivElement>(null);
  const tabIdPrefix = useId();
  const hasTabs = tabs.length > 1;

  const searching = query.trim() !== "";
  const matches = matchSettings(settings, tabs, query);
  const matchedTabIds = new Set<string>(matches.map((s) => s.tab));
  const visibleTabs = searching
    ? tabs.filter((tab) => matchedTabIds.has(tab.id))
    : tabs;
  const activeTab = visibleTabs.some((tab) => tab.id === currentTab)
    ? currentTab
    : visibleTabs[0]?.id;

  const jumpTo = (setting: SettingDefinition) => {
    setCurrentTab(setting.tab);
    setFocusSettingId(setting.id);
    setQuery("");
  };

  useEffect(() => {
    if (!focusSettingId) return;
    const field = tabsRef.current?.querySelector<HTMLElement>(
      `[data-setting-id="${focusSettingId}"]`,
    );
    setFocusSettingId(null);
    if (!field) return;
    field.scrollIntoView({ block: "center" });
    field.querySelector<HTMLElement>("input, select, textarea")?.focus();
  }, [focusSettingId]);

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title={title}
      className="sm:max-w-4xl"
    >
      <ModalBody>
        <ErrorComponent error={error ?? null} />
        {hasTabs && (
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search settings"
            aria-label="Search settings"
            className="w-full mb-3 px-2 py-1.5 rounded-md border border-divider bg-surface text-text focus-ring"
          />
        )}
        {searching && (
          <ul aria-label="Matching settings" className="mb-3 space-y-1">
            {matches.map((setting) => (
              <li key={setting.id}>
                <button
                  type="button"
                  onClick={() => jumpTo(setting)}
                  className="w-full text-left px-2 py-1 rounded hover:bg-surface cursor-pointer flex items-center gap-2 text-sm"
                >
                  <span className="text-primary">{setting.label}</span>
                  <span className="text-tertiary">
                    {tabs.find((t) => t.id === setting.tab)?.label}
                  </span>
                  {setting.restartRequired && <RestartRequiredBadge />}
                </button>
              </li>
            ))}
            {matches.length === 0 && (
              <li className="text-secondary text-sm px-2">
                No settings match "{query.trim()}".
              </li>
            )}
          </ul>
        )}
        {hasTabs && (
          <TabList className="mb-4" label={title} idPrefix={tabIdPrefix}>
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
        <div ref={tabsRef}>
          {tabs.map((tab) =>
            hasTabs ? (
              <TabPanel
                key={tab.id}
                id={tab.id}
                idPrefix={tabIdPrefix}
                active={tab.id === activeTab}
              >
                {tab.content}
              </TabPanel>
            ) : (
              <div key={tab.id}>{tab.content}</div>
            ),
          )}
        </div>
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
