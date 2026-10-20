import React, { useCallback, useContext, useEffect, useState } from "react";
import { Fieldset } from "../forms/Fieldset";
import { BrowseResponse } from "../../api-types";
import { APIContext } from "../../context";
import { BsFolder2Open, BsFolderFill, BsFileEarmark } from "react-icons/bs";
import { GoArrowUp } from "react-icons/go";
import { Spinner } from "../Spinner";
import { RestartNotes } from "./SettingNotes";
import { settingLabel } from "./settingsRegistry";

export interface FoldersTabProps {
  downloadFolder: string;
  completedFolder: string;
  onDownloadFolderChange: (value: string) => void;
  onCompletedFolderChange: (value: string) => void;
}

// Inline directory browser component
const DirectoryBrowser: React.FC<{
  isOpen: boolean;
  initialPath: string;
  onSelect: (path: string) => void;
  onClose: () => void;
}> = ({ isOpen, initialPath, onSelect, onClose }) => {
  const API = useContext(APIContext);
  const [browseData, setBrowseData] = useState<BrowseResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadDirectory = useCallback(
    (path?: string) => {
      setLoading(true);
      setError(null);
      API.browseDirectory(path)
        .then((data) => {
          setBrowseData(data);
        })
        .catch((e) => {
          setError(e?.text || "Failed to browse directory");
        })
        .finally(() => setLoading(false));
    },
    [API],
  );

  useEffect(() => {
    if (isOpen) {
      loadDirectory(initialPath || undefined);
    }
  }, [isOpen, initialPath, loadDirectory]);

  if (!isOpen) return null;

  const dirEntries = browseData?.entries.filter((e) => e.is_dir) ?? [];

  return (
    <div className="border border-divider rounded bg-surface mt-1 overflow-hidden">
      {/* Header with current path and actions */}
      <div className="flex items-center gap-2 px-3 py-2 bg-surface border-b border-divider">
        <span className="text-xs text-secondary truncate flex-1 font-mono">
          {browseData?.current ?? "Loading..."}
        </span>
        <button
          type="button"
          className="text-xs px-2 py-1 rounded bg-primary text-white hover:bg-primary/80 cursor-pointer"
          onClick={() => {
            if (browseData) {
              onSelect(browseData.current);
              onClose();
            }
          }}
          disabled={!browseData}
        >
          Select
        </button>
        <button
          type="button"
          className="text-xs px-2 py-1 rounded bg-surface-raised text-secondary hover:text-text border border-divider cursor-pointer"
          onClick={onClose}
        >
          Cancel
        </button>
      </div>

      {/* Directory listing */}
      <div className="max-h-48 overflow-y-auto">
        {loading && (
          <div className="flex justify-center p-3">
            <Spinner />
          </div>
        )}

        {error && <div className="p-3 text-sm text-error" role="alert">{error}</div>}

        {!loading && !error && browseData && (
          <div className="divide-y divide-divider/50">
            {/* Parent directory */}
            {browseData.parent && (
              <button
                type="button"
                className="flex items-center gap-2 w-full px-3 py-1.5 text-sm text-left hover:bg-surface-raised cursor-pointer"
                onClick={() => loadDirectory(browseData.parent!)}
              >
                <GoArrowUp className="w-4 h-4 text-secondary flex-shrink-0" />
                <span className="text-secondary">..</span>
              </button>
            )}
            {/* Directories */}
            {dirEntries.map((entry) => (
              <button
                key={entry.path}
                type="button"
                className="flex items-center gap-2 w-full px-3 py-1.5 text-sm text-left hover:bg-surface-raised cursor-pointer"
                onClick={() => loadDirectory(entry.path)}
              >
                <BsFolderFill className="w-4 h-4 text-warning flex-shrink-0" />
                <span className="truncate">{entry.name}</span>
              </button>
            ))}
            {dirEntries.length === 0 && !browseData.parent && (
              <div className="px-3 py-2 text-sm text-secondary">
                No subdirectories
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

// Folder input with browse button
const FolderInput: React.FC<{
  setting: string;
  name: string;
  value: string;
  help?: string;
  placeholder?: string;
  onChange: (value: string) => void;
}> = ({ setting, name, value, help, placeholder, onChange }) => {
  const [browserOpen, setBrowserOpen] = useState(false);

  return (
    <div className="flex flex-col gap-2 mb-2" data-setting-id={setting}>
      <label htmlFor={name}>{settingLabel(setting)}</label>
      <div className="flex gap-2">
        <input
          type="text"
          className="flex-1 block border border-divider rounded bg-transparent py-1.5 pl-2 focus-ring focus:border-primary sm:leading-6"
          id={name}
          name={name}
          placeholder={placeholder}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
        <button
          type="button"
          className="flex items-center gap-1 px-3 py-1.5 border border-divider rounded hover:bg-surface-raised text-secondary hover:text-text transition-colors cursor-pointer"
          onClick={() => setBrowserOpen(!browserOpen)}
          title="Browse directories"
        >
          <BsFolder2Open className="w-4 h-4" />
        </button>
      </div>
      {help && <div className="text-sm text-secondary">{help}</div>}
      <DirectoryBrowser
        isOpen={browserOpen}
        initialPath={value}
        onSelect={(path) => onChange(path)}
        onClose={() => setBrowserOpen(false)}
      />
    </div>
  );
};

export const FoldersTab: React.FC<FoldersTabProps> = ({
  downloadFolder,
  completedFolder,
  onDownloadFolderChange,
  onCompletedFolderChange,
}) => {
  return (
    <div className="py-2">
      <Fieldset label="Download Location">
        <FolderInput
          setting="download-folder"
          name="download_folder"
          value={downloadFolder}
          onChange={onDownloadFolderChange}
          help="New torrents will be saved to this folder by default"
          placeholder="/path/to/downloads"
        />
      </Fieldset>
      <Fieldset label="Move on Completion">
        <FolderInput
          setting="completed-folder"
          name="completed_folder"
          value={completedFolder}
          onChange={onCompletedFolderChange}
          help="Finished torrents will be moved here. Leave empty to disable"
          placeholder="/path/to/completed (optional)"
        />
      </Fieldset>
      <div className="mt-4 pt-3 border-t border-divider">
        <RestartNotes tab="folders" />
      </div>
    </div>
  );
};
