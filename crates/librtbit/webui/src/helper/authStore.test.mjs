// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

// The auth store reads localStorage when it is created, so the stand-in must be
// installed before the module is imported.
const backing = new Map();
globalThis.localStorage = {
  getItem: (key) => (backing.has(key) ? backing.get(key) : null),
  setItem: (key, value) => backing.set(key, String(value)),
  removeItem: (key) => backing.delete(key),
  clear: () => backing.clear(),
};
const { useAuthStore } = await import("../stores/authStore.ts");

const signIn = () =>
  useAuthStore.getState().setTokens("access", "refresh", 3600);

test("a fresh store has no expired session", () => {
  assert.equal(useAuthStore.getState().sessionExpired, false);
});

test("expireSession drops the tokens and flags the session as expired", () => {
  signIn();
  useAuthStore.getState().expireSession();

  const state = useAuthStore.getState();
  assert.equal(state.accessToken, null);
  assert.equal(state.refreshToken, null);
  assert.equal(state.state, "login_required");
  assert.equal(state.sessionExpired, true);
  assert.equal(localStorage.getItem("rtbit_access_token"), null);
});

test("a successful login clears the expired flag", () => {
  useAuthStore.getState().expireSession();
  signIn();
  assert.equal(useAuthStore.getState().sessionExpired, false);
  assert.equal(useAuthStore.getState().state, "authenticated");
});

test("expireSession without a held session is not reported as expiry", () => {
  useAuthStore.getState().clearTokens();
  useAuthStore.setState({ sessionExpired: false });

  useAuthStore.getState().expireSession();
  assert.equal(useAuthStore.getState().sessionExpired, false);
  assert.equal(useAuthStore.getState().state, "login_required");
});

test("an explicit logout does not flag the session as expired", () => {
  signIn();
  useAuthStore.getState().clearTokens();
  assert.equal(useAuthStore.getState().sessionExpired, false);
});
