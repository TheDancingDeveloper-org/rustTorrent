import React, { useEffect, useState } from "react";
import { AuthAPI, TokenResponse } from "../../http-api";
import { useAuthStore } from "../../stores/authStore";
// @ts-expect-error - SVG import handled by vite-plugin-svgr
import Logo from "../../../assets/logo.svg?react";

export const LoginPage: React.FC = () => {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const setTokens = useAuthStore((s) => s.setTokens);
  const sessionExpired = useAuthStore((s) => s.sessionExpired);
  const clearSessionExpired = useAuthStore((s) => s.clearSessionExpired);
  // Captured on mount so the notice stays up after the flag is cleared below
  const [expiredNotice] = useState(sessionExpired);

  useEffect(() => {
    if (sessionExpired) clearSessionExpired();
  }, [sessionExpired, clearSessionExpired]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      const tokens: TokenResponse = await AuthAPI.login(username, password);
      setTokens(tokens.access_token, tokens.refresh_token, tokens.expires_in);
    } catch (err: any) {
      setError(err.message || "Login failed");
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
          <p className="text-secondary text-sm mt-1">Sign in to continue</p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          {expiredNotice && !error && (
            <div
              role="status"
              className="bg-warning/10 border border-warning text-primary rounded px-3 py-2 text-sm"
            >
              Your session expired. Sign in again to continue.
            </div>
          )}

          {error && (
            <div className="bg-error-bg/10 border border-error-bg text-error-bg rounded px-3 py-2 text-sm">
              {error}
            </div>
          )}

          <div>
            <label className="block text-sm font-medium text-secondary mb-1">
              Username
            </label>
            <input
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              className={inputClass}
              placeholder="Username"
              autoComplete="username"
              autoFocus
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
              placeholder="Password"
              autoComplete="current-password"
              required
            />
          </div>

          <button
            type="submit"
            disabled={loading || !username || !password}
            className="w-full py-2 bg-primary-bg text-white rounded font-medium hover:bg-primary-bg-hover disabled:opacity-50 transition-colors cursor-pointer disabled:cursor-not-allowed"
          >
            {loading ? "Signing in..." : "Sign In"}
          </button>
        </form>
      </div>
    </div>
  );
};
