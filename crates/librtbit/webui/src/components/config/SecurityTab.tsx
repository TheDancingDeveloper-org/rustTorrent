import React, { useState } from "react";
import { settingLabel } from "./settingsRegistry";
import { AuthAPI } from "../../http-api";
import { useAuthStore } from "../../stores/authStore";

export const SecurityTab: React.FC = () => {
  const [currentPassword, setCurrentPassword] = useState("");
  const [newUsername, setNewUsername] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const authState = useAuthStore((s) => s.state);

  const handleSave = async () => {
    setError(null);
    setSuccess(null);

    if (!currentPassword) {
      setError("Current password is required");
      return;
    }

    if (newPassword && newPassword !== confirmPassword) {
      setError("New passwords do not match");
      return;
    }

    if (newPassword && newPassword.length < 4) {
      setError("New password must be at least 4 characters");
      return;
    }

    if (!newUsername && !newPassword) {
      setError("Enter a new username or password to change");
      return;
    }

    setSaving(true);
    try {
      await AuthAPI.changeCredentials(
        currentPassword,
        newUsername || undefined,
        newPassword || undefined,
      );
      setSuccess("Credentials updated successfully");
      setCurrentPassword("");
      setNewUsername("");
      setNewPassword("");
      setConfirmPassword("");
    } catch (err: any) {
      setError(err.message || "Failed to change credentials");
    } finally {
      setSaving(false);
    }
  };

  const inputClass =
    "w-full px-3 py-2 bg-surface border border-divider rounded text-text placeholder:text-tertiary focus-ring focus:border-primary text-sm";

  if (authState === "no_auth") {
    return (
      <div className="space-y-3 py-2">
        <div className="text-secondary text-sm">
          Authentication is not enabled. Set{" "}
          <code className="text-xs bg-surface px-1 py-0.5 rounded">
            RTBIT_HTTP_BASIC_AUTH_USERPASS
          </code>{" "}
          or use the first-boot setup to enable it.
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4 py-2">
      {error && (
        <div className="bg-error-bg/10 border border-error-bg text-error-bg rounded px-3 py-2 text-sm">
          {error}
        </div>
      )}
      {success && (
        <div className="bg-success-bg/10 border border-success-bg text-success rounded px-3 py-2 text-sm">
          {success}
        </div>
      )}

      <div data-setting-id="current-password">
        <label className="block text-sm font-medium text-secondary mb-1">
          {settingLabel("current-password")}
        </label>
        <input
          type="password"
          value={currentPassword}
          onChange={(e) => setCurrentPassword(e.target.value)}
          className={inputClass}
          placeholder="Enter current password"
          autoComplete="current-password"
        />
      </div>

      <hr className="border-divider" />

      <div data-setting-id="new-username">
        <label className="block text-sm font-medium text-secondary mb-1">
          {settingLabel("new-username")}
        </label>
        <input
          type="text"
          value={newUsername}
          onChange={(e) => setNewUsername(e.target.value)}
          className={inputClass}
          placeholder="Leave blank to keep current"
          autoComplete="username"
        />
      </div>

      <div data-setting-id="new-password">
        <label className="block text-sm font-medium text-secondary mb-1">
          {settingLabel("new-password")}
        </label>
        <input
          type="password"
          value={newPassword}
          onChange={(e) => setNewPassword(e.target.value)}
          className={inputClass}
          placeholder="Leave blank to keep current"
          autoComplete="new-password"
        />
      </div>

      <div data-setting-id="confirm-password">
        <label className="block text-sm font-medium text-secondary mb-1">
          {settingLabel("confirm-password")}
        </label>
        <input
          type="password"
          value={confirmPassword}
          onChange={(e) => setConfirmPassword(e.target.value)}
          className={inputClass}
          placeholder="Confirm new password"
          autoComplete="new-password"
          disabled={!newPassword}
        />
      </div>

      <button
        onClick={handleSave}
        disabled={saving || !currentPassword}
        className="w-full py-2 bg-primary-bg text-white rounded font-medium text-sm hover:bg-primary-bg-hover disabled:opacity-50 transition-colors cursor-pointer disabled:cursor-not-allowed"
      >
        {saving ? "Saving..." : "Update Credentials"}
      </button>
    </div>
  );
};
