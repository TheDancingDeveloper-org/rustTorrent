/** The daemon's error prefix for a stopped torrent whose payload is missing. */
export const MISSING_FILES_ERROR = "payload is missing or incomplete";

/** @param {string | null | undefined} error */
export const isMissingFilesError = (error) =>
  !!error && error.startsWith(MISSING_FILES_ERROR);
