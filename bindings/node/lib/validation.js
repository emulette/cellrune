"use strict";

const { inputError, protocolError } = require("./errors.js");

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

module.exports = {
  requireFinite,
  requireNonNegativeInteger,
  requireObject,
  requireOptionalBoolean,
  requireOptionalFinite,
  requireOptions,
  requireProtocolFinite,
  requireProtocolString,
  requireString,
  requireU64BigInt,
};
