"use strict";

const { inputError, limitPayloadError, protocolError } = require("./errors.js");

function requireObject(value, name) {
  if (value === null || typeof value !== "object") {
    throw protocolError(`${name} must be an object`);
  }
}

function requireString(value, name) {
  if (typeof value !== "string") {
    throw inputError(`${name} must be a string`);
  }
}

function requireFinite(value, name) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw inputError(`${name} must be a finite number`);
  }
}

function requireProtocolFinite(value, name) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw protocolError(`${name} must be a finite number`);
  }
}

function requireProtocolString(value, name) {
  if (typeof value !== "string") {
    throw protocolError(`${name} must be a string`);
  }
}

function requireOptionalFinite(value, name) {
  if (value !== undefined) {
    requireFinite(value, name);
  }
}

function requireOptionalU32(value, name) {
  if (
    value !== undefined &&
    (!Number.isInteger(value) || value < 0 || value > 4294967295)
  ) {
    throw inputError(`${name} must be an unsigned 32-bit integer`);
  }
}

function requireOptionalString(value, name) {
  if (value !== undefined) {
    requireString(value, name);
  }
}

function requireOptionalBoolean(value, name) {
  if (value !== undefined && typeof value !== "boolean") {
    throw inputError(`${name} must be a boolean`);
  }
}

function requireNonNegativeInteger(value, name) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw inputError(`${name} must be a non-negative safe integer`);
  }
}

function requireU64BigInt(value, name) {
  if (
    typeof value !== "bigint" ||
    value < 0n ||
    value > 18446744073709551615n
  ) {
    throw inputError(`${name} must be an unsigned 64-bit bigint`);
  }
}

function requireOptions(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw inputError("options must be an object");
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) {
    throw inputError("options must be a plain object");
  }
}

function requireOptionKeys(value, allowedKeys) {
  for (const key of Object.keys(value)) {
    if (!allowedKeys.includes(key)) {
      throw inputError(`options.${key} is not supported`);
    }
  }
}

/**
 * Converts a camelCase limit object to the snake_case object native code reads, or returns
 * undefined when no limits were given. A value that is not a plain object, a snake_case name,
 * which native code would read as that limit, and values JSON cannot carry unchanged, such as
 * bigints and non-finite numbers, fail here; native code rejects unknown names and values that
 * are not positive integers.
 */
function limitObject(value, name) {
  if (value === undefined || value === null) {
    return undefined;
  }
  if (typeof value !== "object" || Array.isArray(value)) {
    throw limitPayloadError(`${name} must be an object`);
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) {
    throw limitPayloadError(`${name} must be a plain object`);
  }
  const converted = {};
  for (const [key, limit] of Object.entries(value)) {
    if (limit === undefined) {
      continue;
    }
    if (key.includes("_")) {
      throw limitPayloadError(`${name}.${key} is not a limit name`);
    }
    if (typeof limit !== "number" || !Number.isFinite(limit)) {
      throw limitPayloadError(`${name}.${key} must be a finite number`);
    }
    converted[key.replace(/[A-Z]/gu, (letter) => `_${letter.toLowerCase()}`)] = limit;
  }
  return converted;
}

function limitsJson(value, name) {
  const converted = limitObject(value, name);
  return converted === undefined ? null : JSON.stringify(converted);
}

module.exports = {
  limitsJson,
  requireFinite,
  requireNonNegativeInteger,
  requireObject,
  requireOptionalBoolean,
  requireOptionalFinite,
  requireOptionalString,
  requireOptionalU32,
  requireOptionKeys,
  requireOptions,
  requireProtocolFinite,
  requireProtocolString,
  requireString,
  requireU64BigInt,
};
