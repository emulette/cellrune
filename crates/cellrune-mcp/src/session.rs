use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use cellrune_interop::WorkbookSession;
use tokio::sync::Mutex as AsyncMutex;

use crate::error::McpError;

#[derive(Debug, Clone)]
pub(crate) struct SessionHandle {
    workbook: Arc<AsyncMutex<WorkbookSession>>,
}

impl SessionHandle {
    pub(crate) fn workbook(&self) -> &Arc<AsyncMutex<WorkbookSession>> {
        &self.workbook
    }
}

#[derive(Debug)]
struct SessionEntry {
    workbook: Arc<AsyncMutex<WorkbookSession>>,
    last_access: Instant,
}

#[derive(Debug)]
struct SessionState {
    entries: BTreeMap<String, SessionEntry>,
    next_id: u64,
}

/// Thread-safe bounded workbook-session cache with idle TTL and LRU eviction.
#[derive(Debug, Clone)]
pub(crate) struct SessionCache {
    state: Arc<Mutex<SessionState>>,
    maximum: usize,
    ttl: Duration,
}

#[derive(Debug)]
pub(crate) struct PreparedSessionInsert<'a> {
    state: MutexGuard<'a, SessionState>,
    workbook: WorkbookSession,
    evicted_id: Option<String>,
    id: String,
    next_id: u64,
    now: Instant,
}

impl PreparedSessionInsert<'_> {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn commit(mut self) -> SessionHandle {
        if let Some(evicted_id) = &self.evicted_id {
            self.state.entries.remove(evicted_id);
        }
        self.state.next_id = self.next_id;
        let workbook = Arc::new(AsyncMutex::new(self.workbook));
        self.state.entries.insert(
            self.id.clone(),
            SessionEntry {
                workbook: workbook.clone(),
                last_access: self.now,
            },
        );
        SessionHandle { workbook }
    }
}

impl SessionCache {
    pub(crate) fn new(maximum: usize, ttl: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(SessionState {
                entries: BTreeMap::new(),
                next_id: 1,
            })),
            maximum,
            ttl,
        }
    }

    pub(crate) fn prepare_insert(
        &self,
        workbook: WorkbookSession,
    ) -> Result<PreparedSessionInsert<'_>, McpError> {
        self.prepare_insert_at(workbook, Instant::now())
    }

    pub(crate) fn get(&self, id: &str) -> Result<SessionHandle, McpError> {
        self.get_at(id, Instant::now())
    }

    pub(crate) fn touch(&self, id: &str) -> Result<(), McpError> {
        let mut state = self.state.lock().map_err(|_| McpError::session_state())?;
        let entry = state
            .entries
            .get_mut(id)
            .ok_or_else(McpError::session_not_found)?;
        entry.last_access = Instant::now();
        Ok(())
    }

    pub(crate) fn close(&self, id: &str) -> Result<(), McpError> {
        let mut state = self.state.lock().map_err(|_| McpError::session_state())?;
        let entry = state
            .entries
            .get(id)
            .ok_or_else(McpError::session_not_found)?;
        if Arc::strong_count(&entry.workbook) != 1 {
            return Err(McpError::session_busy());
        }
        state.entries.remove(id);
        Ok(())
    }

    pub(crate) fn ids(&self) -> Result<Vec<String>, McpError> {
        let mut state = self.state.lock().map_err(|_| McpError::session_state())?;
        prune_expired(&mut state.entries, Instant::now(), self.ttl);
        Ok(state.entries.keys().cloned().collect())
    }

    fn prepare_insert_at(
        &self,
        workbook: WorkbookSession,
        now: Instant,
    ) -> Result<PreparedSessionInsert<'_>, McpError> {
        let mut state = self.state.lock().map_err(|_| McpError::session_state())?;
        prune_expired(&mut state.entries, now, self.ttl);
        let evicted_id = if state.entries.len() >= self.maximum {
            Some(
                state
                    .entries
                    .iter()
                    .filter(|(_, entry)| Arc::strong_count(&entry.workbook) == 1)
                    .min_by_key(|(id, entry)| (entry.last_access, id.as_str()))
                    .map(|(id, _)| id.clone())
                    .ok_or_else(McpError::session_cache_full)?,
            )
        } else {
            None
        };

        let sequence = state.next_id;
        let next_id = state
            .next_id
            .checked_add(1)
            .ok_or_else(McpError::session_id_exhausted)?;
        let id = format!("workbook-{sequence:016x}");
        Ok(PreparedSessionInsert {
            state,
            workbook,
            evicted_id,
            id,
            next_id,
            now,
        })
    }

    fn get_at(&self, id: &str, now: Instant) -> Result<SessionHandle, McpError> {
        let mut state = self.state.lock().map_err(|_| McpError::session_state())?;
        let expired = state.entries.get(id).is_some_and(|entry| {
            now.saturating_duration_since(entry.last_access) >= self.ttl
                && Arc::strong_count(&entry.workbook) == 1
        });
        if expired {
            state.entries.remove(id);
            return Err(McpError::session_expired());
        }
        prune_expired(&mut state.entries, now, self.ttl);
        let entry = state
            .entries
            .get_mut(id)
            .ok_or_else(McpError::session_not_found)?;
        entry.last_access = now;
        Ok(SessionHandle {
            workbook: entry.workbook.clone(),
        })
    }
}

fn prune_expired(entries: &mut BTreeMap<String, SessionEntry>, now: Instant, ttl: Duration) {
    entries.retain(|_, entry| {
        Arc::strong_count(&entry.workbook) != 1
            || now.saturating_duration_since(entry.last_access) < ttl
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_sessions_return_a_stable_error() {
        let cache = SessionCache::new(2, Duration::from_secs(10));
        let start = Instant::now();
        let handle = cache
            .prepare_insert_at(WorkbookSession::create(), start)
            .expect("session must be inserted");
        let id = handle.id().to_owned();
        let handle = handle.commit();
        drop(handle);

        let error = cache
            .get_at(&id, start + Duration::from_secs(10))
            .expect_err("session must expire at its TTL");

        assert_eq!(error.payload().code, "mcp.session.expired");
    }

    #[test]
    fn least_recently_used_idle_session_is_evicted() {
        let cache = SessionCache::new(2, Duration::from_secs(60));
        let start = Instant::now();
        let first = cache
            .prepare_insert_at(WorkbookSession::create(), start)
            .expect("first session must be inserted");
        let first_id = first.id().to_owned();
        let first = first.commit();
        drop(first);
        let second = cache
            .prepare_insert_at(WorkbookSession::create(), start + Duration::from_secs(1))
            .expect("second session must be inserted");
        let second_id = second.id().to_owned();
        let second = second.commit();
        drop(second);
        let first = cache
            .get_at(&first_id, start + Duration::from_secs(2))
            .expect("first session must be touched");
        drop(first);

        let third = cache
            .prepare_insert_at(WorkbookSession::create(), start + Duration::from_secs(3))
            .expect("third session must evict the LRU entry");
        let third = third.commit();
        drop(third);

        assert!(
            cache
                .get_at(&first_id, start + Duration::from_secs(4))
                .is_ok()
        );
        assert_eq!(
            cache
                .get_at(&second_id, start + Duration::from_secs(4))
                .expect_err("second session must be evicted")
                .payload()
                .code,
            "mcp.session.not_found"
        );
    }

    #[test]
    fn active_sessions_are_not_evicted() {
        let cache = SessionCache::new(1, Duration::from_secs(60));
        let active = cache
            .prepare_insert(WorkbookSession::create())
            .expect("session must be prepared")
            .commit();

        let error = cache
            .prepare_insert(WorkbookSession::create())
            .expect_err("active session must protect its slot");

        assert_eq!(error.payload().code, "mcp.session.cache_full");
        drop(active);
    }
}
