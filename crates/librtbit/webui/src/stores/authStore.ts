import { create } from "zustand";

const TOKEN_KEY = "rtbit_access_token";
const REFRESH_TOKEN_KEY = "rtbit_refresh_token";
const TOKEN_EXPIRY_KEY = "rtbit_token_expiry";

export type AuthState =
  | "loading" // checking auth status
  | "setup_required" // first boot, no credentials
  | "login_required" // credentials exist, need to log in
  | "authenticated" // logged in
  | "no_auth"; // auth not enabled (no credentials, no setup)

export interface AuthStore {
  state: AuthState;
  accessToken: string | null;
  refreshToken: string | null;
  tokenExpiry: number | null;
  // True after the server rejected a session that was held, until a new login
  sessionExpired: boolean;

  setState: (state: AuthState) => void;
  setTokens: (
    accessToken: string,
    refreshToken: string,
    expiresIn: number,
  ) => void;
  clearTokens: () => void;
  expireSession: () => void;
  getAccessToken: () => string | null;
}

export const useAuthStore = create<AuthStore>((set, get) => ({
  state: "loading",
  accessToken: localStorage.getItem(TOKEN_KEY),
  refreshToken: localStorage.getItem(REFRESH_TOKEN_KEY),
  tokenExpiry: (() => {
    const v = localStorage.getItem(TOKEN_EXPIRY_KEY);
    return v ? parseInt(v, 10) : null;
  })(),
  sessionExpired: false,

  setState: (state) => set({ state }),

  setTokens: (accessToken, refreshToken, expiresIn) => {
    const tokenExpiry = Date.now() + expiresIn * 1000;
    localStorage.setItem(TOKEN_KEY, accessToken);
    localStorage.setItem(REFRESH_TOKEN_KEY, refreshToken);
    localStorage.setItem(TOKEN_EXPIRY_KEY, tokenExpiry.toString());
    set({
      accessToken,
      refreshToken,
      tokenExpiry,
      state: "authenticated",
      sessionExpired: false,
    });
  },

  clearTokens: () => {
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(REFRESH_TOKEN_KEY);
    localStorage.removeItem(TOKEN_EXPIRY_KEY);
    set({
      accessToken: null,
      refreshToken: null,
      tokenExpiry: null,
      state: "login_required",
    });
  },

  // Drops the tokens because the server no longer accepts them. Unlike a
  // logout, this records that the user was signed in and got kicked out.
  expireSession: () => {
    const { accessToken, refreshToken } = get();
    const hadSession = accessToken !== null || refreshToken !== null;
    get().clearTokens();
    if (hadSession) set({ sessionExpired: true });
  },

  getAccessToken: () => {
    const { accessToken, tokenExpiry } = get();
    if (!accessToken || !tokenExpiry) return null;
    // Return null if expired (with 30s buffer)
    if (Date.now() > tokenExpiry - 30000) return null;
    return accessToken;
  },
}));
