import React, { useState } from "react";
import { Fieldset } from "../forms/Fieldset";
import { FormCheckbox } from "../forms/FormCheckbox";
import { QueueTab, QueueTabProps } from "./QueueTab";
import { SeedingTab } from "./SeedingTab";
import { settingLabel } from "./settingsRegistry";
import {
  DoubleClickAction,
  loadDoubleClickAction,
  loadStartPaused,
  saveDoubleClickAction,
  saveStartPaused,
} from "../../helper/behaviourPrefs";

const DOUBLE_CLICK_LABELS: Record<DoubleClickAction, string> = {
  details: "Open torrent details",
  toggle: "Start or pause the torrent",
  none: "Do nothing",
};

export const BehaviourTab: React.FC<QueueTabProps> = (queueProps) => {
  const [startPaused, setStartPaused] = useState(loadStartPaused);
  const [doubleClick, setDoubleClick] = useState(loadDoubleClickAction);

  return (
    <div className="py-2">
      <QueueTab {...queueProps} />
      <SeedingTab />

      <Fieldset label="Adding Torrents">
        <FormCheckbox
          checked={startPaused}
          setting="add-start-paused"
          name="add_start_paused"
          help="New torrents are added paused. The add dialog can still override it per torrent. Saved in this browser."
          onChange={() => {
            setStartPaused(!startPaused);
            saveStartPaused(!startPaused);
          }}
        />
      </Fieldset>

      <Fieldset label="Torrent List">
        <div className="flex flex-col gap-1">
          <label
            htmlFor="double_click_action"
            className="text-sm font-medium text-secondary"
          >
            {settingLabel("double-click-action")}
          </label>
          <select
            id="double_click_action"
            data-setting-id="double-click-action"
            value={doubleClick}
            onChange={(e) => {
              const value = e.target.value as DoubleClickAction;
              setDoubleClick(value);
              saveDoubleClickAction(value);
            }}
            className="px-2 py-1.5 text-sm bg-surface border border-divider rounded focus-ring focus:border-primary"
          >
            {(Object.keys(DOUBLE_CLICK_LABELS) as DoubleClickAction[]).map(
              (action) => (
                <option key={action} value={action}>
                  {DOUBLE_CLICK_LABELS[action]}
                </option>
              ),
            )}
          </select>
          <span className="text-secondary text-xs">
            What double-clicking a row in the torrent list does. Saved in this
            browser.
          </span>
        </div>
      </Fieldset>
    </div>
  );
};
