import React, { useContext, useEffect, useState } from "react";
import { Fieldset } from "../forms/Fieldset";
import { FormInput } from "../forms/FormInput";
import { FormCheckbox } from "../forms/FormCheckbox";
import { formatBytes } from "../../helper/formatBytes";
import { APIContext } from "../../context";
import { AltSpeedConfig, AltSpeedSchedule } from "../../api-types";

export interface RateLimitsTabProps {
  downloadBps: number | null | undefined;
  uploadBps: number | null | undefined;
  peerLimit: number | null | undefined;
  concurrentInitLimit: number | null | undefined;
  onDownloadBpsChange: (value: number | null) => void;
  onUploadBpsChange: (value: number | null) => void;
  onPeerLimitChange: (value: number | null) => void;
  onConcurrentInitLimitChange: (value: number | null) => void;
}

const formatLimitHelp = (
  bps: number | null | undefined,
  label: string,
): string => {
  const value = bps ?? 0;
  if (value > 0) {
    return `Limit total ${label} speed to this number of bytes per second (current ${formatBytes(value)} per second)`;
  }
  return `Limit total ${label} speed to this number of bytes per second (currently disabled)`;
};

const DAY_LABELS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const DAY_BITS = [1, 2, 4, 8, 16, 32, 64];

const minutesToTimeStr = (minutes: number): string => {
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return `${h.toString().padStart(2, "0")}:${m.toString().padStart(2, "0")}`;
};

const timeStrToMinutes = (time: string): number => {
  const [h, m] = time.split(":").map(Number);
  return (h || 0) * 60 + (m || 0);
};

export const RateLimitsTab: React.FC<RateLimitsTabProps> = ({
  downloadBps,
  uploadBps,
  peerLimit,
  concurrentInitLimit,
  onDownloadBpsChange,
  onUploadBpsChange,
  onPeerLimitChange,
  onConcurrentInitLimitChange,
}) => {
  const API = useContext(APIContext);

  const [altEnabled, setAltEnabled] = useState(false);
  const [altConfig, setAltConfig] = useState<AltSpeedConfig>({
    download_rate: null,
    upload_rate: null,
  });
  const [schedule, setSchedule] = useState<AltSpeedSchedule>({
    enabled: false,
    start_minutes: 0,
    end_minutes: 0,
    days: 0,
  });
  const [altLoaded, setAltLoaded] = useState(false);
  const [altError, setAltError] = useState<string | null>(null);

  useEffect(() => {
    API.getAltSpeed()
      .then((status) => {
        setAltEnabled(status.enabled);
        setAltConfig(status.config);
        if (status.schedule) {
          setSchedule(status.schedule);
        }
        setAltLoaded(true);
      })
      .catch(() => {
        // Server doesn't support alternative speed limits — hide the section
        // instead of offering a toggle that can't be saved.
        setAltLoaded(false);
      });
  }, [API]);

  const handleAltToggle = async (enabled: boolean) => {
    setAltEnabled(enabled);
    setAltError(null);
    try {
      await API.toggleAltSpeed(enabled);
    } catch {
      // revert on error and tell the user why the checkbox snapped back
      setAltEnabled(!enabled);
      setAltError("Could not update alternative speed limits on the server.");
    }
  };

  const handleAltConfigSave = async (config: AltSpeedConfig) => {
    setAltConfig(config);
    try {
      await API.setAltSpeedConfig(config);
    } catch {
      // silent
    }
  };

  const handleScheduleSave = async (sched: AltSpeedSchedule) => {
    setSchedule(sched);
    try {
      await API.setSpeedSchedule(sched);
    } catch {
      // silent
    }
  };

  return (
    <>
      <Fieldset label="Speed Limits">
        <FormInput
          setting="download-rate-limit"
          name="download_bps"
          inputType="number"
          value={downloadBps?.toString() ?? ""}
          onChange={(e) => {
            const val = e.target.valueAsNumber;
            onDownloadBpsChange(isNaN(val) || val <= 0 ? null : val);
          }}
          help={formatLimitHelp(downloadBps, "download")}
        />
        <FormInput
          setting="upload-rate-limit"
          name="upload_bps"
          inputType="number"
          value={uploadBps?.toString() ?? ""}
          onChange={(e) => {
            const val = e.target.valueAsNumber;
            onUploadBpsChange(isNaN(val) || val <= 0 ? null : val);
          }}
          help={formatLimitHelp(uploadBps, "upload")}
        />
        <FormInput
          setting="peer-limit"
          name="peer_limit"
          inputType="number"
          value={peerLimit?.toString() ?? ""}
          onChange={(e) => {
            const val = e.target.valueAsNumber;
            onPeerLimitChange(isNaN(val) || val <= 0 ? null : val);
          }}
          help={`Maximum number of peers per torrent (current: ${peerLimit ?? "default"})`}
        />
        <FormInput
          setting="concurrent-init-limit"
          name="concurrent_init_limit"
          inputType="number"
          value={concurrentInitLimit?.toString() ?? ""}
          onChange={(e) => {
            const val = e.target.valueAsNumber;
            onConcurrentInitLimitChange(isNaN(val) || val <= 0 ? null : val);
          }}
          help={`Maximum number of torrents initializing concurrently (current: ${concurrentInitLimit ?? "default"})`}
        />
      </Fieldset>

      {altLoaded && (
        <Fieldset label="Alternative Speed Limits">
          <FormCheckbox
            checked={altEnabled}
            setting="alt-speed-enabled"
            name="alt_speed_enabled"
            help="When enabled, alternative speed limits override the normal limits above."
            onChange={() => handleAltToggle(!altEnabled)}
          />
          {altError && <p className="text-sm text-error mt-1">{altError}</p>}
          {altEnabled && (
            <div className="mt-3 space-y-2 ml-6">
              <FormInput
                setting="alt-download-limit"
                name="alt_download_rate"
                inputType="number"
                value={
                  altConfig.download_rate != null
                    ? String(Math.round(altConfig.download_rate / 1024))
                    : ""
                }
                onChange={(e) => {
                  const val = e.target.valueAsNumber;
                  const newConfig = {
                    ...altConfig,
                    download_rate: isNaN(val) || val <= 0 ? null : val * 1024,
                  };
                  handleAltConfigSave(newConfig);
                }}
                help="Alternative download speed limit in KB/s (0 or empty = unlimited)"
              />
              <FormInput
                setting="alt-upload-limit"
                name="alt_upload_rate"
                inputType="number"
                value={
                  altConfig.upload_rate != null
                    ? String(Math.round(altConfig.upload_rate / 1024))
                    : ""
                }
                onChange={(e) => {
                  const val = e.target.valueAsNumber;
                  const newConfig = {
                    ...altConfig,
                    upload_rate: isNaN(val) || val <= 0 ? null : val * 1024,
                  };
                  handleAltConfigSave(newConfig);
                }}
                help="Alternative upload speed limit in KB/s (0 or empty = unlimited)"
              />
            </div>
          )}

          <div className="mt-4">
            <FormCheckbox
              checked={schedule.enabled}
              setting="speed-schedule"
              name="alt_schedule_enabled"
              help="Automatically enable alternative speed limits during scheduled times."
              onChange={() => {
                handleScheduleSave({
                  ...schedule,
                  enabled: !schedule.enabled,
                });
              }}
            />
            {schedule.enabled && (
              <div className="mt-3 ml-6 space-y-3">
                <div className="flex gap-4 items-center">
                  <div className="flex flex-col gap-1">
                    <label
                      htmlFor="schedule_start"
                      className="text-sm text-secondary"
                    >
                      Start time
                    </label>
                    <input
                      type="time"
                      id="schedule_start"
                      className="block border border-divider rounded bg-transparent py-1.5 pl-2 pr-2 focus:ring-0 focus:border-primary sm:leading-6"
                      value={minutesToTimeStr(schedule.start_minutes)}
                      onChange={(e) => {
                        handleScheduleSave({
                          ...schedule,
                          start_minutes: timeStrToMinutes(e.target.value),
                        });
                      }}
                    />
                  </div>
                  <div className="flex flex-col gap-1">
                    <label
                      htmlFor="schedule_end"
                      className="text-sm text-secondary"
                    >
                      End time
                    </label>
                    <input
                      type="time"
                      id="schedule_end"
                      className="block border border-divider rounded bg-transparent py-1.5 pl-2 pr-2 focus:ring-0 focus:border-primary sm:leading-6"
                      value={minutesToTimeStr(schedule.end_minutes)}
                      onChange={(e) => {
                        handleScheduleSave({
                          ...schedule,
                          end_minutes: timeStrToMinutes(e.target.value),
                        });
                      }}
                    />
                  </div>
                </div>
                <div>
                  <label className="text-sm text-secondary block mb-2">
                    Days
                  </label>
                  <div className="flex gap-3 flex-wrap">
                    {DAY_LABELS.map((day, i) => (
                      <label
                        key={day}
                        className="flex items-center gap-1 text-sm cursor-pointer"
                      >
                        <input
                          type="checkbox"
                          checked={(schedule.days & DAY_BITS[i]) !== 0}
                          onChange={() => {
                            handleScheduleSave({
                              ...schedule,
                              days: schedule.days ^ DAY_BITS[i],
                            });
                          }}
                        />
                        {day}
                      </label>
                    ))}
                  </div>
                </div>
              </div>
            )}
          </div>
        </Fieldset>
      )}
    </>
  );
};
