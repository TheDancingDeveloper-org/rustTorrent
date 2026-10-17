import React, { useState } from "react";
import { AuthAPI, TokenResponse } from "../../http-api";
import { useAuthStore } from "../../stores/authStore";
// @ts-expect-error - SVG import handled by vite-plugin-svgr
import Logo from "../../../assets/logo.svg?react";

export const SetupPage: React.FC = () => {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  // The desktop app injects the token into its own webview; in a browser this is absent.
  const [setupToken, setSetupToken] = useState(
    () =>
      (window as unknown as { __RTBIT_SETUP_TOKEN__?: string })
        .__RTBIT_SETUP_TOKEN__ ?? "",
  );
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const setTokens = useAuthStore((s) => s.setTokens);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (password !== confirmPassword) {
      setError("Passwords do not match");
      return;
    }

    if (password.length < 4) {
      setError("Password must be at least 4 characters");
      return;
    }

    setLoading(true);
    try {
      const tokens: TokenResponse = await AuthAPI.setup(
        username,
        password,
        setupToken.trim(),
      );
      setTokens(tokens.access_token, tokens.refresh_token, tokens.expires_in);
    } catch (err: any) {
      setError(err.message || "Setup failed");
    } finally {
      setLoading(false);
    }
  };

  const inputClass =
    "w-full px-3 py-2 bg-surface border border-divider rounded text-primary placeholder:text-tertiary focus:outline-none focus:border-primary";

  return (
    <div className="bg-surface h-dvh flex items-center justify-center">
      <div className="bg-surface-raised shadow-lg rounded-lg p-8 w-full max-w-sm">
        <div className="flex flex-col items-center mb-6">
          <Logo className="w-12 h-12 mb-2" alt="RustTorrent" />
          <h1 className="text-2xl font-bold">RustTorrent</h1>
          <p className="text-secondary text-sm mt-1">First-time setup</p>
          <p className="text-tertiary text-xs mt-2 text-center">
            Create a username and password to secure your RustTorrent instance.
          </p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          {error && (
            <div className="bg-error-bg/10 border border-error-bg text-error-bg rounded px-3 py-2 text-sm">
              {error}
            </div>
          )}

          <div>
            <label className="block text-sm font-medium text-secondary mb-1">
              Setup token
            </label>
            <input
              type="text"
              value={setupToken}
              onChange={(e) => setSetupToken(e.target.value)}
              className={inputClass}
              placeholder="Paste the token from the rtbit log"
              autoComplete="off"
              spellCheck={false}
              autoFocus
              required
            />
            <p className="text-tertiary text-xs mt-1">
              Printed in the rtbit server log at startup (for example{" "}
              <code>docker logs</code>).
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium text-secondary mb-1">
              Username
            </label>
            <input
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              className={inputClass}
              placeholder="Choose a username"
              autoComplete="username"
              required
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-secondary mb-1">
              Password
            </label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className={inputClass}
              placeholder="Choose a password"
              autoComplete="new-password"
              required
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-secondary mb-1">
              Confirm Password
            </label>
            <input
              type="password"
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
              className={inputClass}
              placeholder="Confirm your password"
              autoComplete="new-password"
              required
            />
          </div>

          <button
            type="submit"
            disabled={
              loading || !setupToken || !username || !password || !confirmPassword
            }
            className="w-full py-2 bg-primary-bg text-white rounded font-medium hover:bg-primary-bg-hover disabled:opacity-50 transition-colors cursor-pointer disabled:cursor-not-allowed"
          >
            {loading ? "Creating account..." : "Create Account"}
          </button>
        </form>
      </div>
    </div>
  );
};
