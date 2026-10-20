import { useEffect, useState } from "react";
import { FaArrowLeft, FaCheck, FaCopy } from "react-icons/fa";

import { IndexarrAPI } from "../http-api";
import { useIndexarrStore } from "../stores/indexarrStore";
import { useErrorStore } from "../stores/errorStore";
import { Spinner } from "./Spinner";

export const IndexarrSetup = () => {
  const identity = useIndexarrStore((s) => s.identity);
  const setIdentity = useIndexarrStore((s) => s.setIdentity);
  const syncPreferences = useIndexarrStore((s) => s.syncPreferences);
  const setSyncPreferences = useIndexarrStore((s) => s.setSyncPreferences);
  const setShowSetup = useIndexarrStore((s) => s.setShowSetup);
  const setCloseableError = useErrorStore((s) => s.setCloseableError);

  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);
  const [selectedCategories, setSelectedCategories] = useState<string[]>([]);
  const [syncComments, setSyncComments] = useState(true);
  const [step, setStep] = useState<"identity" | "categories">("identity");

  // Load identity + preferences on mount
  useEffect(() => {
    (async () => {
      try {
        const [id, prefs] = await Promise.all([
          IndexarrAPI.getIdentityStatus(),
          IndexarrAPI.getSyncPreferences(),
        ]);
        setIdentity(id);
        setSyncPreferences(prefs);
        setSelectedCategories(prefs.import_categories);
        setSyncComments(prefs.sync_comments);

        // Skip identity step if already acknowledged
        if (!id.needs_onboarding) {
          setStep("categories");
        }
      } catch (e: any) {
        setCloseableError({
          text: "Failed to load Indexarr setup",
          details: e,
        });
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  const handleCopyKey = async () => {
    if (identity?.recovery_key) {
      try {
        await navigator.clipboard.writeText(identity.recovery_key);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      } catch {
        // fallback: select textarea
      }
    }
  };

  const handleAcknowledge = async () => {
    setSaving(true);
    try {
      await IndexarrAPI.acknowledgeIdentity();
      const updated = await IndexarrAPI.getIdentityStatus();
      setIdentity(updated);
      setStep("categories");
    } catch (e: any) {
      setCloseableError({
        text: "Failed to acknowledge identity",
        details: e,
      });
    } finally {
      setSaving(false);
    }
  };

  const toggleCategory = (cat: string) => {
    setSelectedCategories((prev) =>
      prev.includes(cat) ? prev.filter((c) => c !== cat) : [...prev, cat],
    );
  };

  const handleSavePreferences = async () => {
    setSaving(true);
    try {
      const result = await IndexarrAPI.setSyncPreferences({
        import_categories: selectedCategories,
        sync_comments: syncComments,
      });
      setSyncPreferences(result);
      setShowSetup(false);
    } catch (e: any) {
      setCloseableError({
        text: "Failed to save sync preferences",
        details: e,
      });
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Spinner label="Loading setup" />
      </div>
    );
  }

  return (
    <div className="h-full flex items-center justify-center p-4">
      <div className="bg-surface-raised rounded-lg shadow-lg max-w-lg w-full">
        {/* Header */}
        <div className="flex items-center gap-2 p-4 border-b border-divider">
          {identity && !identity.needs_onboarding && (
            <button
              onClick={() => setShowSetup(false)}
              className="p-1 text-secondary hover:text-text cursor-pointer"
            >
              <FaArrowLeft className="w-4 h-4" />
            </button>
          )}
          <h2 className="text-lg font-semibold">Indexarr Setup</h2>
        </div>

        {step === "identity" && (
          <div className="p-4 space-y-4">
            <p className="text-sm text-secondary">
              Your Indexarr node has generated a unique contributor identity.
              Save the recovery key below — you will need it to restore your
              identity if you reinstall.
            </p>

            {identity?.contributor_id && (
              <div>
                <label className="text-xs font-medium text-secondary mb-1 block">
                  Contributor ID
                </label>
                <code className="block text-sm bg-surface px-3 py-2 rounded font-mono">
                  {identity.contributor_id}
                </code>
              </div>
            )}

            {identity?.recovery_key && (
              <div>
                <label className="text-xs font-medium text-secondary mb-1 block">
                  Recovery Key
                </label>
                <div className="relative">
                  <textarea
                    readOnly
                    value={identity.recovery_key}
                    className="w-full text-sm bg-surface px-3 py-2 rounded font-mono resize-none h-20 border border-divider"
                  />
                  <button
                    onClick={handleCopyKey}
                    className="absolute top-2 right-2 p-1.5 bg-surface-raised rounded hover:bg-primary/10 cursor-pointer"
                    title="Copy to clipboard"
                  >
                    {copied ? (
                      <FaCheck className="w-3.5 h-3.5 text-success" />
                    ) : (
                      <FaCopy className="w-3.5 h-3.5 text-secondary" />
                    )}
                  </button>
                </div>
                <p className="text-xs text-tertiary mt-1">
                  Store this key securely. It cannot be recovered later.
                </p>
              </div>
            )}

            <button
              onClick={handleAcknowledge}
              disabled={saving}
              className="w-full py-2 rounded bg-primary text-white font-medium hover:bg-primary/80 disabled:opacity-50 cursor-pointer"
            >
              {saving ? "Saving..." : "I have saved my recovery key"}
            </button>
          </div>
        )}

        {step === "categories" && (
          <div className="p-4 space-y-4">
            <p className="text-sm text-secondary">
              Select which content categories to sync from the network.
            </p>

            <div className="grid grid-cols-2 gap-2">
              {(syncPreferences?.all_categories || []).map((cat) => (
                <label
                  key={cat}
                  className="flex items-center gap-2 px-3 py-2 bg-surface rounded cursor-pointer hover:bg-surface-raised/50"
                >
                  <input
                    type="checkbox"
                    checked={selectedCategories.includes(cat)}
                    onChange={() => toggleCategory(cat)}
                    className="rounded"
                  />
                  <span className="text-sm capitalize">
                    {cat.replace("_", " ")}
                  </span>
                </label>
              ))}
            </div>

            <label className="flex items-center gap-2 px-3 py-2 bg-surface rounded cursor-pointer">
              <input
                type="checkbox"
                checked={syncComments}
                onChange={(e) => setSyncComments(e.target.checked)}
                className="rounded"
              />
              <span className="text-sm">Sync comments</span>
            </label>

            <button
              onClick={handleSavePreferences}
              disabled={saving || selectedCategories.length === 0}
              className="w-full py-2 rounded bg-primary text-white font-medium hover:bg-primary/80 disabled:opacity-50 cursor-pointer"
            >
              {saving ? "Saving..." : "Save Preferences"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
};
