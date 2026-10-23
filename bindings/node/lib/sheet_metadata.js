"use strict";

const { requireObject } = require("./validation.js");

function page(value, name, normalizeEntry) {
  requireObject(value, name);
  return {
    schemaVersion: value.schemaVersion,
    sheet: value.sheet,
    total: value.total,
    offset: value.offset,
    nextOffset: value.nextOffset ?? null,
    entries: value.entries.map(normalizeEntry),
  };
}

function normalizeDataValidation(rule) {
  return {
    ranges: rule.ranges,
    validationType: rule.validationType,
    operator: rule.operator,
    formula1: rule.formula1 ?? null,
    formula2: rule.formula2 ?? null,
    allowBlank: rule.allowBlank,
    inCellDropdown: rule.inCellDropdown,
    showInputMessage: rule.showInputMessage,
    showErrorMessage: rule.showErrorMessage,
    errorStyle: rule.errorStyle,
    errorTitle: rule.errorTitle ?? null,
    errorMessage: rule.errorMessage ?? null,
    promptTitle: rule.promptTitle ?? null,
    promptMessage: rule.promptMessage ?? null,
  };
}

function normalizeDataValidationPage(value) {
  return page(value, "data validation page", normalizeDataValidation);
}

function normalizeHyperlinkPage(value) {
  return page(value, "hyperlink page", (link) => ({
    range: link.range,
    target: link.target ?? null,
    location: link.location ?? null,
    display: link.display ?? null,
    tooltip: link.tooltip ?? null,
  }));
}

function normalizeCommentPage(value) {
  return page(value, "comment page", (comment) => ({
    address: comment.address,
    author: comment.author ?? null,
    text: comment.text,
  }));
}

module.exports = {
  normalizeCommentPage,
  normalizeDataValidationPage,
  normalizeHyperlinkPage,
};
