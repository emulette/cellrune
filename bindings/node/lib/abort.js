"use strict";

const native = require("../native.js");
const { inputError, withErrors, withSyncErrors } = require("./errors.js");

function requireOptionalAbortSignal(signal) {
  if (signal !== undefined && !(signal instanceof AbortSignal)) {
    throw inputError("signal must be an AbortSignal");
  }
}

/**
 * Starts a native calculation with a cancellation handle that `signal` aborts. An aborted signal
 * cancels the calculation cooperatively, so the Promise rejects with the calculation's own
 * cancellation error, and a signal aborted before the call cancels it before it calculates.
 */
async function runAbortable(signal, start) {
  if (signal === undefined) {
    return withErrors(withSyncErrors(() => start(null)));
  }
  const cancellation = new native.NativeCancellation();
  const abort = () => cancellation.cancel();
  if (signal.aborted) {
    abort();
  } else {
    signal.addEventListener("abort", abort, { once: true });
  }
  try {
    return await withErrors(withSyncErrors(() => start(cancellation)));
  } finally {
    signal.removeEventListener("abort", abort);
  }
}

module.exports = { requireOptionalAbortSignal, runAbortable };
