// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { MAX_TOASTS, useToastStore } from "../stores/toastStore.ts";

const reset = () => useToastStore.setState({ toasts: [] });

test("pushToast appends toasts in order with distinct ids", () => {
  reset();
  const { pushToast } = useToastStore.getState();
  pushToast("info", "first");
  pushToast("error", "second");

  const { toasts } = useToastStore.getState();
  assert.deepEqual(
    toasts.map((t) => [t.tone, t.text]),
    [
      ["info", "first"],
      ["error", "second"],
    ],
  );
  assert.notEqual(toasts[0].id, toasts[1].id);
});

test("the oldest toast is dropped once the stack is full", () => {
  reset();
  const { pushToast } = useToastStore.getState();
  for (let i = 0; i < MAX_TOASTS + 2; i++) pushToast("info", `t${i}`);

  const texts = useToastStore.getState().toasts.map((t) => t.text);
  assert.equal(texts.length, MAX_TOASTS);
  assert.equal(texts[0], `t${2}`);
  assert.equal(texts.at(-1), `t${MAX_TOASTS + 1}`);
});

test("dismissToast removes only the given toast", () => {
  reset();
  const { pushToast, dismissToast } = useToastStore.getState();
  pushToast("success", "keep");
  pushToast("success", "drop");
  const dropId = useToastStore.getState().toasts[1].id;

  dismissToast(dropId);
  assert.deepEqual(
    useToastStore.getState().toasts.map((t) => t.text),
    ["keep"],
  );
});

test("dismissing an unknown id changes nothing", () => {
  reset();
  useToastStore.getState().pushToast("info", "only");
  useToastStore.getState().dismissToast(-1);
  assert.equal(useToastStore.getState().toasts.length, 1);
});
