import { create } from "zustand";

export type ToastTone = "info" | "success" | "error";

export interface Toast {
  id: number;
  tone: ToastTone;
  text: string;
}

/** Most toasts shown at once; the oldest is dropped when a new one overflows. */
export const MAX_TOASTS = 4;

export interface ToastStore {
  toasts: Toast[];
  pushToast: (tone: ToastTone, text: string) => void;
  dismissToast: (id: number) => void;
}

let nextToastId = 1;

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],

  pushToast: (tone, text) => {
    const toast: Toast = { id: nextToastId++, tone, text };
    set({ toasts: [...get().toasts, toast].slice(-MAX_TOASTS) });
  },

  dismissToast: (id) => {
    set({ toasts: get().toasts.filter((t) => t.id !== id) });
  },
}));
