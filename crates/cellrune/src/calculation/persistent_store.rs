use std::sync::{Arc, OnceLock};

use super::performance_counters::{WorkCounter, work_counter_add};

const KEY_BYTES: usize = size_of::<u128>();

/// Every radix leaf owns one fixed-size immutable handle. Payload bytes live behind that handle,
/// so even a value larger than this nominal byte cap remains a singleton shared payload rather
/// than forcing an unbounded leaf clone.
pub(crate) const LEAF_ENTRY_CAPACITY: usize = 1;
pub(crate) const LEAF_OWNED_BYTE_CAPACITY: usize = 64;

#[derive(Debug)]
struct RadixNode<V> {
    children: Vec<(u8, Arc<RadixNode<V>>)>,
    value: Option<V>,
    semantic_fingerprint: OnceLock<[u8; 32]>,
}

struct RadixFrame<'a, V> {
    node: &'a RadixNode<V>,
    key: u128,
    depth: usize,
    next_child: usize,
    value_yielded: bool,
}

struct RadixDfs<'a, V> {
    frames: Vec<RadixFrame<'a, V>>,
    reverse: bool,
}

impl<'a, V> RadixDfs<'a, V> {
    fn new(root: &'a RadixNode<V>, reverse: bool) -> Self {
        let mut frames = Vec::with_capacity(KEY_BYTES + 1);
        frames.push(RadixFrame {
            node: root,
            key: 0,
            depth: 0,
            next_child: 0,
            value_yielded: false,
        });
        Self { frames, reverse }
    }

    fn next(&mut self) -> Option<(u128, &'a V)> {
        loop {
            let frame = self.frames.last_mut()?;
            if !frame.value_yielded {
                frame.value_yielded = true;
                if let Some(value) = frame.node.value.as_ref() {
                    return Some((frame.key, value));
                }
            }
            if frame.next_child == frame.node.children.len() {
                self.frames.pop();
                continue;
            }
            let child_index = if self.reverse {
                frame.node.children.len() - frame.next_child - 1
            } else {
                frame.next_child
            };
            frame.next_child += 1;
            let (edge, child) = &frame.node.children[child_index];
            let key = (frame.key << 8) | u128::from(*edge);
            let depth = frame.depth + 1;
            self.frames.push(RadixFrame {
                node: child.as_ref(),
                key,
                depth,
                next_child: 0,
                value_yielded: false,
            });
        }
    }
}

pub(crate) struct PersistentRadixEntries<'a, V> {
    front: RadixDfs<'a, V>,
    back: RadixDfs<'a, V>,
    remaining: usize,
}

impl<'a, V> Iterator for PersistentRadixEntries<'a, V> {
    type Item = (u128, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        self.front.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<V> DoubleEndedIterator for PersistentRadixEntries<'_, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        self.back.next()
    }
}

impl<V> ExactSizeIterator for PersistentRadixEntries<'_, V> {}

pub(crate) struct PersistentRadixValues<'a, V> {
    entries: PersistentRadixEntries<'a, V>,
}

impl<'a, V> Iterator for PersistentRadixValues<'a, V> {
    type Item = &'a V;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next().map(|(_, value)| value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl<V> DoubleEndedIterator for PersistentRadixValues<'_, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.next_back().map(|(_, value)| value)
    }
}

impl<V> ExactSizeIterator for PersistentRadixValues<'_, V> {}

impl<V> Default for RadixNode<V> {
    fn default() -> Self {
        Self {
            children: Vec::new(),
            value: None,
            semantic_fingerprint: OnceLock::new(),
        }
    }
}

/// A fixed-depth, canonical persistent radix map for internal snapshot stores.
///
/// Keys are traversed in big-endian byte order. Consequently iteration is numeric-key ordered,
/// every mutation copies at most seventeen nodes, and the tree shape is independent of insertion or
/// edit history. Values are expected to be cheap immutable handles such as `Arc<_>`.
#[derive(Debug, Clone)]
pub(crate) struct PersistentRadixMap<V> {
    root: Arc<RadixNode<V>>,
    len: usize,
}

impl<V> Default for PersistentRadixMap<V> {
    fn default() -> Self {
        Self {
            root: Arc::new(RadixNode::default()),
            len: 0,
        }
    }
}

impl<V: Clone> PersistentRadixMap<V> {
    pub(crate) fn get(&self, key: u128) -> Option<&V> {
        let bytes = key.to_be_bytes();
        let mut node = self.root.as_ref();
        for byte in bytes {
            let index = node
                .children
                .binary_search_by_key(&byte, |(edge, _)| *edge)
                .ok()?;
            node = node.children[index].1.as_ref();
        }
        node.value.as_ref()
    }

    /// Inserts a value and returns the previous value plus the exact number of newly allocated
    /// radix nodes. The latter is deterministic structural-copy evidence, not payload cloning.
    pub(crate) fn insert(&mut self, key: u128, value: V) -> (Option<V>, u64) {
        assert_eq!(LEAF_ENTRY_CAPACITY, 1, "radix leaves are singleton");
        assert!(
            size_of::<V>() <= LEAF_OWNED_BYTE_CAPACITY,
            "radix leaf handle exceeds its owned-byte capacity"
        );
        let bytes = key.to_be_bytes();
        let mut copied = 0;
        let (root, previous) = insert_node(&self.root, &bytes, 0, value, &mut copied);
        self.root = root;
        if previous.is_none() {
            self.len += 1;
        }
        (previous, copied)
    }

    /// Removes a value and returns it plus the exact number of newly allocated radix nodes.
    pub(crate) fn remove(&mut self, key: u128) -> (Option<V>, u64) {
        let bytes = key.to_be_bytes();
        let mut copied = 0;
        let (root, removed) = remove_node(&self.root, &bytes, 0, &mut copied);
        if let Some(root) = root {
            self.root = root;
        } else {
            self.root = Arc::new(RadixNode::default());
        }
        if removed.is_some() {
            self.len -= 1;
        }
        (removed, copied)
    }

    /// Streams values in ascending numeric-key order with a stack bounded by the fixed key depth.
    pub(crate) fn ordered_values(&self) -> PersistentRadixValues<'_, V> {
        PersistentRadixValues {
            entries: self.ordered_entries(),
        }
    }

    pub(crate) fn ordered_entries(&self) -> PersistentRadixEntries<'_, V> {
        PersistentRadixEntries {
            front: RadixDfs::new(self.root.as_ref(), false),
            back: RadixDfs::new(self.root.as_ref(), true),
            remaining: self.len,
        }
    }

    pub(crate) fn semantic_fingerprint_cancellable(
        &self,
        leaf_fingerprint: &impl Fn(&V) -> Result<[u8; 32], ()>,
        internal_fingerprint: &impl Fn(usize, &[(u8, [u8; 32])]) -> [u8; 32],
        cancelled: &impl Fn() -> bool,
    ) -> Result<[u8; 32], ()> {
        fingerprint_node(
            self.root.as_ref(),
            0,
            leaf_fingerprint,
            internal_fingerprint,
            cancelled,
        )
    }
}

fn insert_node<V: Clone>(
    node: &Arc<RadixNode<V>>,
    key: &[u8; KEY_BYTES],
    depth: usize,
    value: V,
    copied: &mut u64,
) -> (Arc<RadixNode<V>>, Option<V>) {
    *copied += 1;
    if depth == KEY_BYTES {
        return (
            Arc::new(RadixNode {
                children: node.children.clone(),
                value: Some(value),
                semantic_fingerprint: OnceLock::new(),
            }),
            node.value.clone(),
        );
    }

    let edge = key[depth];
    let mut children = node.children.clone();
    let previous = match children.binary_search_by_key(&edge, |(candidate, _)| *candidate) {
        Ok(index) => {
            let (child, previous) = insert_node(&children[index].1, key, depth + 1, value, copied);
            children[index].1 = child;
            previous
        }
        Err(index) => {
            let empty = Arc::new(RadixNode::default());
            let (child, previous) = insert_node(&empty, key, depth + 1, value, copied);
            children.insert(index, (edge, child));
            previous
        }
    };
    (
        Arc::new(RadixNode {
            children,
            value: node.value.clone(),
            semantic_fingerprint: OnceLock::new(),
        }),
        previous,
    )
}

fn remove_node<V: Clone>(
    node: &Arc<RadixNode<V>>,
    key: &[u8; KEY_BYTES],
    depth: usize,
    copied: &mut u64,
) -> (Option<Arc<RadixNode<V>>>, Option<V>) {
    if depth == KEY_BYTES {
        let Some(removed) = node.value.clone() else {
            return (Some(Arc::clone(node)), None);
        };
        *copied += 1;
        if node.children.is_empty() {
            return (None, Some(removed));
        }
        return (
            Some(Arc::new(RadixNode {
                children: node.children.clone(),
                value: None,
                semantic_fingerprint: OnceLock::new(),
            })),
            Some(removed),
        );
    }

    let edge = key[depth];
    let Ok(index) = node
        .children
        .binary_search_by_key(&edge, |(candidate, _)| *candidate)
    else {
        return (Some(Arc::clone(node)), None);
    };
    let (child, removed) = remove_node(&node.children[index].1, key, depth + 1, copied);
    if removed.is_none() {
        return (Some(Arc::clone(node)), None);
    }
    *copied += 1;
    let mut children = node.children.clone();
    match child {
        Some(child) => children[index].1 = child,
        None => {
            children.remove(index);
        }
    }
    if children.is_empty() && node.value.is_none() {
        (None, removed)
    } else {
        (
            Some(Arc::new(RadixNode {
                children,
                value: node.value.clone(),
                semantic_fingerprint: OnceLock::new(),
            })),
            removed,
        )
    }
}

fn fingerprint_node<V>(
    node: &RadixNode<V>,
    depth: usize,
    leaf_fingerprint: &impl Fn(&V) -> Result<[u8; 32], ()>,
    internal_fingerprint: &impl Fn(usize, &[(u8, [u8; 32])]) -> [u8; 32],
    cancelled: &impl Fn() -> bool,
) -> Result<[u8; 32], ()> {
    if let Some(fingerprint) = node.semantic_fingerprint.get() {
        work_counter_add(WorkCounter::FingerprintCachedNodesReused, 1);
        return Ok(*fingerprint);
    }
    if cancelled() {
        return Err(());
    }
    let fingerprint = if depth == KEY_BYTES {
        leaf_fingerprint(node.value.as_ref().expect("radix leaf contains a value"))?
    } else {
        let mut children = Vec::with_capacity(node.children.len());
        for (edge, child) in &node.children {
            if cancelled() {
                return Err(());
            }
            children.push((
                *edge,
                fingerprint_node(
                    child.as_ref(),
                    depth + 1,
                    leaf_fingerprint,
                    internal_fingerprint,
                    cancelled,
                )?,
            ));
        }
        work_counter_add(WorkCounter::FingerprintInternalNodesHashed, 1);
        internal_fingerprint(depth, &children)
    };
    let _ = node.semantic_fingerprint.set(fingerprint);
    Ok(*node
        .semantic_fingerprint
        .get()
        .expect("radix fingerprint was initialized"))
}

#[cfg(test)]
mod tests {
    use super::PersistentRadixMap;

    #[test]
    fn map_is_ordered_and_history_independent() {
        let mut ascending = PersistentRadixMap::default();
        for key in [1, 257, 2, u64::MAX] {
            ascending.insert(u128::from(key), key);
        }
        assert_eq!(
            ascending.ordered_values().copied().collect::<Vec<_>>(),
            vec![1, 2, 257, u64::MAX]
        );

        let mut descending = PersistentRadixMap::default();
        for key in [u64::MAX, 2, 257, 1] {
            descending.insert(u128::from(key), key);
        }
        let leaf = |value: &u64| {
            let mut digest = [0_u8; 32];
            digest[..8].copy_from_slice(&value.to_be_bytes());
            Ok(digest)
        };
        let internal = |depth: usize, children: &[(u8, [u8; 32])]| {
            let mut output = [0_u8; 32];
            output[0] = depth as u8;
            for (edge, digest) in children {
                output[1] ^= *edge;
                output[2] ^= digest[0];
            }
            output
        };
        assert_eq!(
            ascending.semantic_fingerprint_cancellable(&leaf, &internal, &|| false),
            descending.semantic_fingerprint_cancellable(&leaf, &internal, &|| false)
        );
    }

    #[test]
    fn streaming_iterator_supports_mixed_front_and_back_consumption() {
        let mut map = PersistentRadixMap::default();
        for key in [257_u128, 1, u128::MAX, 2] {
            map.insert(key, key);
        }
        let mut entries = map.ordered_entries();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries.next().map(|(key, _)| key), Some(1));
        assert_eq!(entries.next_back().map(|(key, _)| key), Some(u128::MAX));
        assert_eq!(entries.next().map(|(key, _)| key), Some(2));
        assert_eq!(entries.next_back().map(|(key, _)| key), Some(257));
        assert_eq!(entries.next(), None);
        assert_eq!(entries.next_back(), None);
    }
}
