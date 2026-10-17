import React from "react";
import { Fieldset } from "../forms/Fieldset";
import { FormInput } from "../forms/FormInput";

export interface QueueTabProps {
  maxActiveDownloads: number | null | undefined;
  maxActiveUploads: number | null | undefined;
  maxActiveTotal: number | null | undefined;
  onMaxActiveDownloadsChange: (value: number | null) => void;
  onMaxActiveUploadsChange: (value: number | null) => void;
  onMaxActiveTotalChange: (value: number | null) => void;
}

export const QueueTab: React.FC<QueueTabProps> = ({
  maxActiveDownloads,
  maxActiveUploads,
  maxActiveTotal,
  onMaxActiveDownloadsChange,
  onMaxActiveUploadsChange,
  onMaxActiveTotalChange,
}) => {
  return (
    <div className="py-2">
      <Fieldset label="Queue Limits">
        <FormInput
          setting="max-active-downloads"
          name="max_active_downloads"
          inputType="number"
          value={maxActiveDownloads?.toString() ?? ""}
          onChange={(e) => {
            const val = e.target.valueAsNumber;
            onMaxActiveDownloadsChange(isNaN(val) || val < 0 ? null : val);
          }}
          help="Maximum number of active downloading torrents (0 or empty = unlimited)"
        />
        <FormInput
          setting="max-active-uploads"
          name="max_active_uploads"
          inputType="number"
          value={maxActiveUploads?.toString() ?? ""}
          onChange={(e) => {
            const val = e.target.valueAsNumber;
            onMaxActiveUploadsChange(isNaN(val) || val < 0 ? null : val);
          }}
          help="Maximum number of active seeding torrents (0 or empty = unlimited)"
        />
        <FormInput
          setting="max-active-total"
          name="max_active_total"
          inputType="number"
          value={maxActiveTotal?.toString() ?? ""}
          onChange={(e) => {
            const val = e.target.valueAsNumber;
            onMaxActiveTotalChange(isNaN(val) || val < 0 ? null : val);
          }}
          help="Maximum number of active torrents total (0 or empty = unlimited)"
        />
      </Fieldset>
    </div>
  );
};
