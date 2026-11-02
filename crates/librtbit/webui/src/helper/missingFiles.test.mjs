// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import { isMissingFilesError } from "./missingFiles.js";

test("isMissingFilesError matches the daemon flag with a path suffix", () => {
  assert.equal(
    isMissingFilesError(
      "payload is missing or incomplete on read-only storage: a/b.bin",
    ),
    true,
  );
  assert.equal(isMissingFilesError("low disk space"), false);
  assert.equal(isMissingFilesError(null), false);
});
