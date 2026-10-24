// Per-browser behaviour preferences from the Settings > Behaviour tab. They
// only change what this web UI does, so they live in localStorage beside the
// view preferences rather than in the engine.

import { isBoolean, loadViewPref, oneOf, saveViewPref } from "./viewPrefs";

export const DOUBLE_CLICK_ACTIONS = ["details", "toggle", "none"] as const;
export type DoubleClickAction = (typeof DOUBLE_CLICK_ACTIONS)[number];

const START_PAUSED_KEY = "behaviour.startPaused";
const DOUBLE_CLICK_KEY = "behaviour.doubleClick";

export const loadStartPaused = (): boolean =>
  loadViewPref(START_PAUSED_KEY, false, isBoolean);

export const saveStartPaused = (value: boolean): void =>
  saveViewPref(START_PAUSED_KEY, value);

export const loadDoubleClickAction = (): DoubleClickAction =>
  loadViewPref(DOUBLE_CLICK_KEY, "details", oneOf(DOUBLE_CLICK_ACTIONS));

export const saveDoubleClickAction = (value: DoubleClickAction): void =>
  saveViewPref(DOUBLE_CLICK_KEY, value);
