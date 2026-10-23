use std::collections::BTreeMap;

use super::super::xml::{
    DOCUMENT_RELATIONSHIPS_STRICT, DOCUMENT_RELATIONSHIPS_TRANSITIONAL, XmlAttributes,
};
use super::table::parse_reference;
use crate::{CellRange, Hyperlink};

const HYPERLINKS: &[u8] = b"hyperlinks";
const HYPERLINK: &[u8] = b"hyperlink";

/// One `<hyperlink>` whose relationship target is resolved after the worksheet is read.
#[derive(Debug)]
pub(super) struct PendingHyperlink {
    range: CellRange,
    relationship_id: Option<Box<str>>,
    location: Option<Box<str>>,
    display: Option<Box<str>>,
    tooltip: Option<Box<str>>,
}

impl PendingHyperlink {
    pub(super) fn has_relationship(&self) -> bool {
        self.relationship_id.is_some()
    }

    /// Resolves the external target from the worksheet's hyperlink relationships.
    pub(super) fn resolve(self, targets: &BTreeMap<Box<str>, Box<str>>) -> Hyperlink {
        let external = self.relationship_id.is_some();
        let target = self
            .relationship_id
            .and_then(|id| targets.get(&id).cloned());
        Hyperlink::from_xlsx(
            self.range,
            external,
            target,
            self.location,
            [self.display, self.tooltip],
        )
    }
}

/// Collects the worksheet's `<hyperlinks>`. A hyperlink whose `ref` does not parse is left
/// unmodeled; its XML is preserved either way.
#[derive(Debug, Default)]
pub(super) struct HyperlinkCollector {
    open_depth: Option<u64>,
    hyperlinks: Vec<PendingHyperlink>,
}

impl HyperlinkCollector {
    /// Records one SpreadsheetML start or empty element.
    pub(super) fn start(
        &mut self,
        local_name: &[u8],
        depth: u64,
        empty: bool,
        attributes: &XmlAttributes,
    ) {
        if depth == 2 && local_name == HYPERLINKS && !empty {
            self.open_depth = Some(depth);
        } else if self.open_depth.is_some_and(|parent| depth == parent + 1)
            && local_name == HYPERLINK
            && let Some(range) = attributes.unqualified("ref").and_then(parse_reference)
        {
            let text = |name| attributes.unqualified(name).map(Box::from);
            self.hyperlinks.push(PendingHyperlink {
                range,
                relationship_id: attributes
                    .namespaced(DOCUMENT_RELATIONSHIPS_TRANSITIONAL, "id")
                    .or_else(|| attributes.namespaced(DOCUMENT_RELATIONSHIPS_STRICT, "id"))
                    .map(Box::from),
                location: text("location"),
                display: text("display"),
                tooltip: text("tooltip"),
            });
        }
    }

    /// Closes `<hyperlinks>` when `depth` is its depth.
    pub(super) fn end(&mut self, depth: u64) {
        if self.open_depth == Some(depth) {
            self.open_depth = None;
        }
    }

    pub(super) fn finish(self) -> Vec<PendingHyperlink> {
        self.hyperlinks
    }
}
