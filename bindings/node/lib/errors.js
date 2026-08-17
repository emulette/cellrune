"use strict";

const ERROR_PREFIX = "CELLRUNE_ERROR:";
const ERROR = Object.freeze({
  CLOSED: ["interop.session.closed", "workbook session is closed"],
  ARGUMENT: ["interop.input.invalid", "argument failed runtime validation"],
  PROTOCOL: [
    "interop.binding.protocol",
    "native binding returned an invalid typed value",
  ],
});
const PROTOCOL_DETAIL = Object.freeze({
  PREVIEW_PAYLOAD_MALFORMED: "native preview payload is malformed",
  PREVIEW_PAYLOAD_JSON_INVALID: "native preview payload is invalid JSON",
  TRANSACTION_DETAIL_KIND_UNKNOWN: "transaction detail kind is unknown",
  TRANSACTION_RESULT_MALFORMED: "transaction calculation result is malformed",
});
const INPUT_DETAIL = Object.freeze({
  PREVIEW_SECTION_INVALID: "section is not a transaction detail section",
  PREVIEW_CURSOR_INVALID: "cursor must be a preview cursor object",
});

class CellRuneError extends Error {
  constructor(message, code, kind, details = {}) {
    super(message);
    this.name = "CellRuneError";
    this.code = code;
    this.kind = kind;
    this.details = {
      sourceCode: details.sourceCode ?? null,
      sourceId: details.sourceId ?? null,
      detail: details.detail ?? null,
    };
  }
}

function withSyncErrors(operation) {
  try {
    return operation();
  } catch (error) {
    throw convertError(error);
  }
}

async function withErrors(promise) {
  try {
    return await promise;
  } catch (error) {
    throw convertError(error);
  }
}

function convertError(error) {
  if (error instanceof CellRuneError) {
    return error;
  }
  const message = error instanceof Error ? error.message : String(error);
  const marker = message.indexOf(ERROR_PREFIX);
  if (marker >= 0) {
    try {
      const payload = JSON.parse(message.slice(marker + ERROR_PREFIX.length));
      if (
        typeof payload.code === "string" &&
        typeof payload.kind === "string" &&
        typeof payload.message === "string" &&
        payload.details &&
        typeof payload.details === "object"
      ) {
        return new CellRuneError(payload.message, payload.code, payload.kind, {
          sourceCode: payload.details.source_code,
          sourceId: payload.details.source_id,
          detail: payload.details.detail,
        });
      }
    } catch {
      return protocolError("native error payload is malformed");
    }
  }
  return error instanceof Error ? error : new Error(message);
}

function closedError() {
  return new CellRuneError(ERROR.CLOSED[1], ERROR.CLOSED[0], "state");
}

function inputError(detail) {
  return new CellRuneError(ERROR.ARGUMENT[1], ERROR.ARGUMENT[0], "input", {
    detail,
  });
}

function protocolError(detail) {
  return new CellRuneError(ERROR.PROTOCOL[1], ERROR.PROTOCOL[0], "state", {
    detail,
  });
}

module.exports = {
  CellRuneError,
  INPUT_DETAIL,
  PROTOCOL_DETAIL,
  closedError,
  inputError,
  protocolError,
  withErrors,
  withSyncErrors,
};
