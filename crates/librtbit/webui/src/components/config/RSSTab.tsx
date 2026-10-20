import React, { useEffect, useState } from "react";
import { settingLabel } from "./settingsRegistry";
import { RssAPI } from "../../http-api";

interface RSSTabProps {
  rssHistoryLimit: number | null;
  onRssHistoryLimitChange: (value: number | null) => void;
}

export const RSSTab: React.FC<RSSTabProps> = ({
  rssHistoryLimit,
  onRssHistoryLimitChange,
}) => {
  const [feedCount, setFeedCount] = useState<number | null>(null);
  const [ruleCount, setRuleCount] = useState<number | null>(null);
  const [itemCount, setItemCount] = useState<number | null>(null);

  useEffect(() => {
    RssAPI.getFeeds()
      .then((f) => setFeedCount(f.length))
      .catch(() => {});
    RssAPI.getRules()
      .then((r) => setRuleCount(r.length))
      .catch(() => {});
    RssAPI.getItems(undefined, 1)
      .then(() => RssAPI.getItems().then((items) => setItemCount(items.length)))
      .catch(() => {});
  }, []);

  const inputCls =
    "w-full max-w-32 px-2 py-1.5 text-sm bg-surface border border-divider rounded focus-ring focus:border-primary";
  const labelCls = "text-sm text-secondary";
  const rowCls = "flex items-center justify-between py-2";
  const sectionCls = "border-b border-divider pb-3 mb-3";

  return (
    <div className="space-y-1">
      {/* Stats section */}
      <div className={sectionCls}>
        <h3 className="text-xs font-semibold uppercase text-tertiary mb-2">
          RSS Status
        </h3>
        <div className="grid grid-cols-3 gap-3">
          <div className="text-center">
            <div className="text-lg font-semibold">{feedCount ?? "-"}</div>
            <div className="text-xs text-secondary">Feeds</div>
          </div>
          <div className="text-center">
            <div className="text-lg font-semibold">{ruleCount ?? "-"}</div>
            <div className="text-xs text-secondary">Rules</div>
          </div>
          <div className="text-center">
            <div className="text-lg font-semibold">{itemCount ?? "-"}</div>
            <div className="text-xs text-secondary">Items</div>
          </div>
        </div>
      </div>

      {/* Settings */}
      <div>
        <h3 className="text-xs font-semibold uppercase text-tertiary mb-2">
          Settings
        </h3>
        <div className={rowCls} data-setting-id="feed-history-limit">
          <label className={labelCls}>
            {settingLabel("feed-history-limit")}
          </label>
          <input
            type="number"
            className={inputCls}
            value={rssHistoryLimit ?? ""}
            onChange={(e) => {
              const val = e.target.value;
              onRssHistoryLimitChange(val === "" ? null : Number(val));
            }}
            placeholder="Unlimited"
            min={0}
          />
        </div>
        <p className="text-xs text-tertiary mt-1">
          Maximum number of RSS feed items to keep in history. Empty =
          unlimited. Default: 500.
        </p>
      </div>
    </div>
  );
};
