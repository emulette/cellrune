"use strict";

const { inputError } = require("./errors.js");
const {
  requireNonNegativeInteger,
  requireObject,
  requireOptions,
  requireString,
} = require("./validation.js");

const PROPERTY_KEYS = {
  fontId: "font_id",
  phoneticType: "phonetic_type",
  alignment: "alignment",
};

function requireKeys(value, allowed, path) {
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) {
      throw inputError(`${path}.${key} is not supported`);
    }
  }
}

function serializeRuns(runs, path) {
  if (!Array.isArray(runs)) {
    throw inputError(`${path} must be an array of phonetic runs`);
  }
  return runs.map((run, index) => {
    const runPath = `${path}[${index}]`;
    requireOptions(run);
    requireKeys(run, ["startUtf16", "endUtf16", "text"], runPath);
    requireNonNegativeInteger(run.startUtf16, `${runPath}.startUtf16`);
    requireNonNegativeInteger(run.endUtf16, `${runPath}.endUtf16`);
    requireString(run.text, `${runPath}.text`);
    return { start_utf16: run.startUtf16, end_utf16: run.endUtf16, text: run.text };
  });
}

function serializeProperties(properties, path) {
  if (properties === undefined) {
    return undefined;
  }
  requireOptions(properties);
  const serialized = {};
  for (const [key, value] of Object.entries(properties)) {
    const target = PROPERTY_KEYS[key];
    if (target === undefined) {
      throw inputError(`${path}.${key} is not supported`);
    }
    serialized[target] = value;
  }
  return serialized;
}

/**
 * Serializes a `setFrozenPane`, `setPhonetics`, or `clearPhonetics` change, or returns
 * `undefined` for any other kind.
 */
function serializePresentationChange(change, index) {
  const path = `changes[${index}]`;
  switch (change.kind) {
    case "setFrozenPane":
      requireKeys(change, ["kind", "sheet", "rows", "columns"], path);
      requireString(change.sheet, `${path}.sheet`);
      requireNonNegativeInteger(change.rows, `${path}.rows`);
      requireNonNegativeInteger(change.columns, `${path}.columns`);
      return {
        kind: "set_frozen_pane",
        sheet: change.sheet,
        rows: change.rows,
        columns: change.columns,
      };
    case "setPhonetics": {
      requireKeys(
        change,
        ["kind", "sheet", "address", "runs", "visible", "properties"],
        path,
      );
      requireString(change.sheet, `${path}.sheet`);
      requireString(change.address, `${path}.address`);
      const serialized = {
        kind: "set_phonetics",
        sheet: change.sheet,
        address: change.address,
        runs: serializeRuns(change.runs, `${path}.runs`),
        properties: serializeProperties(change.properties, `${path}.properties`),
      };
      if (change.visible !== undefined) {
        serialized.visible = change.visible;
      }
      return serialized;
    }
    case "clearPhonetics":
      requireKeys(change, ["kind", "sheet", "address"], path);
      requireString(change.sheet, `${path}.sheet`);
      requireString(change.address, `${path}.address`);
      return {
        kind: "clear_phonetics",
        sheet: change.sheet,
        address: change.address,
      };
    default:
      return undefined;
  }
}

/** Converts a native phonetic page, sent as JSON, to camel-case fields. */
function normalizePhoneticPage(json) {
  const page = JSON.parse(json);
  requireObject(page, "phonetic page");
  return {
    schemaVersion: page.schema_version,
    sheet: page.sheet,
    total: page.total,
    offset: page.offset,
    nextOffset: page.next_offset ?? null,
    entries: page.entries.map((entry) => ({
      address: entry.address,
      runs: entry.runs.map((run) => ({
        startUtf16: run.start_utf16,
        endUtf16: run.end_utf16,
        text: run.text,
      })),
      properties:
        entry.properties == null
          ? null
          : {
              fontId: entry.properties.font_id,
              phoneticType: entry.properties.phonetic_type ?? null,
              alignment: entry.properties.alignment ?? null,
            },
      visible: entry.visible,
    })),
  };
}

module.exports = { normalizePhoneticPage, serializePresentationChange };
