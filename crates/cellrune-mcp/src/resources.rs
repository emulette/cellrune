use rmcp::model::{ListResourceTemplatesResult, ListResourcesResult, Resource, ResourceTemplate};
use serde::Serialize;

use crate::error::McpError;

pub(crate) const FUNCTION_CATALOG_URI: &str = "cellrune://support/functions";
pub(crate) const SESSION_RESOURCE_PREFIX: &str = "cellrune://sessions/";
const SESSION_RESOURCE_SUFFIX: &str = "/summary";
const SESSION_RESOURCE_TEMPLATE: &str = "cellrune://sessions/{session_id}/summary";
const JSON_MIME_TYPE: &str = "application/json";
const CURSOR_CATALOG: &str = "cellrune-resources-v1:catalog";
const CURSOR_SESSION_PREFIX: &str = "cellrune-resources-v1:session:";
const SESSION_ID_PREFIX: &str = "workbook-";
const SESSION_ID_HEX_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResourcePosition {
    Catalog,
    Session(String),
}

impl ResourcePosition {
    fn parse(cursor: &str) -> Result<Self, McpError> {
        if cursor == CURSOR_CATALOG {
            return Ok(Self::Catalog);
        }
        let session_id = cursor
            .strip_prefix(CURSOR_SESSION_PREFIX)
            .ok_or_else(McpError::resource_cursor_invalid)?;
        if valid_session_id(session_id) {
            Ok(Self::Session(session_id.to_owned()))
        } else {
            Err(McpError::resource_cursor_invalid())
        }
    }

    fn cursor(&self) -> String {
        match self {
            Self::Catalog => CURSOR_CATALOG.to_owned(),
            Self::Session(session_id) => format!("{CURSOR_SESSION_PREFIX}{session_id}"),
        }
    }
}

pub(crate) fn list_resources_page(
    mut session_ids: Vec<String>,
    cursor: Option<&str>,
    maximum_bytes: usize,
) -> Result<ListResourcesResult, McpError> {
    session_ids.sort();
    session_ids.dedup();
    let position = cursor.map(ResourcePosition::parse).transpose()?;
    let mut items = Vec::with_capacity(session_ids.len() + usize::from(position.is_none()));
    if position.is_none() {
        items.push((ResourcePosition::Catalog, function_catalog_resource()));
    }
    let after_session = match &position {
        Some(ResourcePosition::Session(session_id)) => Some(session_id.as_str()),
        _ => None,
    };
    items.extend(
        session_ids
            .into_iter()
            .filter(|session_id| after_session.is_none_or(|after| session_id.as_str() > after))
            .map(|session_id| {
                (
                    ResourcePosition::Session(session_id.clone()),
                    session_resource(&session_id),
                )
            }),
    );

    bounded_page(items, maximum_bytes)
}

pub(crate) fn list_resource_templates_page(
    cursor: Option<&str>,
    maximum_bytes: usize,
) -> Result<ListResourceTemplatesResult, McpError> {
    if cursor.is_some() {
        return Err(McpError::resource_cursor_invalid());
    }
    let result = ListResourceTemplatesResult::with_all_items(vec![
        ResourceTemplate::new(
            SESSION_RESOURCE_TEMPLATE,
            "cellrune_workbook_session_summary",
        )
        .with_title("CellRune workbook session summary")
        .with_description("Bounded metadata for an active opaque workbook session")
        .with_mime_type(JSON_MIME_TYPE),
    ]);
    require_size(&result, maximum_bytes)?;
    Ok(result)
}

pub(crate) fn session_resource_uri(session_id: &str) -> String {
    format!("{SESSION_RESOURCE_PREFIX}{session_id}{SESSION_RESOURCE_SUFFIX}")
}

pub(crate) fn session_id_from_resource(uri: &str) -> Option<&str> {
    let value = uri
        .strip_prefix(SESSION_RESOURCE_PREFIX)?
        .strip_suffix(SESSION_RESOURCE_SUFFIX)?;
    (!value.is_empty() && !value.contains('/')).then_some(value)
}

fn bounded_page(
    items: Vec<(ResourcePosition, Resource)>,
    maximum_bytes: usize,
) -> Result<ListResourcesResult, McpError> {
    if items.is_empty() {
        let result = ListResourcesResult::with_all_items(Vec::new());
        require_size(&result, maximum_bytes)?;
        return Ok(result);
    }

    let item_count = items.len();
    let mut resources = Vec::with_capacity(item_count);
    let mut accepted = None;
    for (index, (position, resource)) in items.into_iter().enumerate() {
        resources.push(resource);
        let next_cursor = (index + 1 < item_count).then(|| position.cursor());
        let candidate = ListResourcesResult {
            meta: None,
            next_cursor,
            resources: resources.clone(),
        };
        match serialized_size(&candidate) {
            Ok(actual_bytes) if actual_bytes <= maximum_bytes => accepted = Some(candidate),
            Ok(actual_bytes) => {
                return accepted.ok_or_else(|| {
                    McpError::response_too_large(actual_bytes as u64, maximum_bytes as u64)
                });
            }
            Err(error) => return Err(error),
        }
    }

    accepted.ok_or_else(|| McpError::response_too_large(0, maximum_bytes as u64))
}

fn function_catalog_resource() -> Resource {
    Resource::new(FUNCTION_CATALOG_URI, "cellrune_function_catalog")
        .with_title("CellRune function support")
        .with_description("Versioned catalog of accepted calculation function names")
        .with_mime_type(JSON_MIME_TYPE)
}

fn session_resource(session_id: &str) -> Resource {
    Resource::new(
        session_resource_uri(session_id),
        format!("{session_id}_summary"),
    )
    .with_title(format!("Workbook session {session_id}"))
    .with_description("Bounded metadata summary for one active workbook session")
    .with_mime_type(JSON_MIME_TYPE)
}

fn valid_session_id(value: &str) -> bool {
    value.strip_prefix(SESSION_ID_PREFIX).is_some_and(|hex| {
        hex.len() == SESSION_ID_HEX_LEN
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn require_size<T: Serialize>(value: &T, maximum_bytes: usize) -> Result<(), McpError> {
    let actual_bytes = serialized_size(value)?;
    if actual_bytes > maximum_bytes {
        Err(McpError::response_too_large(
            actual_bytes as u64,
            maximum_bytes as u64,
        ))
    } else {
        Ok(())
    }
}

fn serialized_size<T: Serialize>(value: &T) -> Result<usize, McpError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| McpError::serialization(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn session_id(sequence: u64) -> String {
        format!("workbook-{sequence:016x}")
    }

    #[test]
    fn resource_pages_are_byte_bounded_and_lossless() {
        let session_ids = (1..=40).map(session_id).collect::<Vec<_>>();
        let expected = session_ids
            .iter()
            .map(|id| session_resource_uri(id))
            .chain(std::iter::once(FUNCTION_CATALOG_URI.to_owned()))
            .collect::<BTreeSet<_>>();
        let mut cursor = None;
        let mut found = BTreeSet::new();
        let mut page_count = 0;

        loop {
            let page = list_resources_page(session_ids.clone(), cursor.as_deref(), 1_024)
                .expect("bounded resource page must be produced");
            assert!(serialized_size(&page).expect("page must serialize") <= 1_024);
            assert!(!page.resources.is_empty());
            for resource in page.resources {
                assert!(
                    found.insert(resource.uri),
                    "resource must not repeat across pages"
                );
            }
            page_count += 1;
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        assert!(page_count > 1);
        assert_eq!(found, expected);
    }

    #[test]
    fn keyset_cursor_survives_a_removed_session() {
        let first = session_id(1);
        let second = session_id(2);
        let third = session_id(3);
        let cursor = ResourcePosition::Session(second.clone()).cursor();

        let page = list_resources_page(vec![first, third.clone()], Some(&cursor), 1_024)
            .expect("removed cursor item must not invalidate a keyset page");

        assert_eq!(
            page.resources
                .into_iter()
                .map(|resource| resource.uri)
                .collect::<Vec<_>>(),
            vec![session_resource_uri(&third)]
        );
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn malformed_resource_cursor_is_rejected() {
        let error = list_resources_page(Vec::new(), Some("not-a-cellrune-cursor"), 1_024)
            .expect_err("malformed cursor must fail");

        assert_eq!(error.payload().code, "mcp.resource.cursor_invalid");
    }

    #[test]
    fn a_page_that_cannot_fit_one_item_fails_closed() {
        let error = list_resources_page(Vec::new(), None, 16)
            .expect_err("an unrepresentable page must fail");

        assert_eq!(error.payload().code, "mcp.response.byte_limit_exceeded");
    }

    #[test]
    fn template_list_enforces_the_same_byte_limit() {
        let error = list_resource_templates_page(None, 16)
            .expect_err("an unrepresentable template list must fail");

        assert_eq!(error.payload().code, "mcp.response.byte_limit_exceeded");
    }
}
