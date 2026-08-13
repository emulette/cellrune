use std::mem::size_of;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

const KEY_BYTES: usize = size_of::<u128>();
const CANCELLATION_POLL_INTERVAL: usize = 256;

/// A leaf owns at most this many key/immutable-block handles.
pub(crate) const LEAF_ENTRY_CAPACITY: usize = 256;
/// Inline keys, block handles, indices, and immutable value blocks are capped independently from
/// heap payloads owned behind values such as strings and arrays.
pub(crate) const LEAF_OWNED_BYTE_CAPACITY: usize = 32 * 1024;

#[derive(Debug)]
struct LeafEntry<V> {
    key: u128,
    values: Arc<[V]>,
    index: usize,
}

impl<V> LeafEntry<V> {
    fn singleton(key: u128, value: V) -> Self {
        Self {
            key,
            values: Arc::from(vec![value]),
            index: 0,
        }
    }

    fn value(&self) -> &V {
        &self.values[self.index]
    }

    fn shared_value(&self) -> PersistentValue<V> {
        PersistentValue {
            values: Arc::clone(&self.values),
            index: self.index,
        }
    }
}

impl<V> Clone for LeafEntry<V> {
    fn clone(&self) -> Self {
        Self {
            key: self.key,
            values: Arc::clone(&self.values),
            index: self.index,
        }
    }
}

/// One immutable value handle returned by a persistent mutation.
#[derive(Debug)]
pub(crate) struct PersistentValue<V> {
    values: Arc<[V]>,
    index: usize,
}

impl<V> Deref for PersistentValue<V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        &self.values[self.index]
    }
}

#[derive(Debug)]
enum RadixNodeKind<V> {
    Branch {
        depth: u8,
        children: Vec<(u8, Arc<RadixNode<V>>)>,
    },
    Leaf(Vec<LeafEntry<V>>),
}

#[derive(Debug)]
struct RadixNode<V> {
    kind: RadixNodeKind<V>,
    len: usize,
    min_key: u128,
    semantic_fingerprint: OnceLock<[u8; 32]>,
}

impl<V> RadixNode<V> {
    fn leaf(entries: Vec<LeafEntry<V>>) -> Self {
        Self {
            len: entries.len(),
            min_key: entries.first().map_or(0, |entry| entry.key),
            kind: RadixNodeKind::Leaf(entries),
            semantic_fingerprint: OnceLock::new(),
        }
    }

    fn branch(depth: usize, children: Vec<(u8, Arc<Self>)>, len: usize) -> Self {
        Self {
            min_key: children
                .first()
                .expect("radix branch is non-empty")
                .1
                .min_key,
            kind: RadixNodeKind::Branch {
                depth: depth as u8,
                children,
            },
            len,
            semantic_fingerprint: OnceLock::new(),
        }
    }
}

/// Borrowed ordered contents of one packed leaf.
pub(crate) struct PersistentRadixLeaf<'a, V> {
    entries: &'a [LeafEntry<V>],
}

impl<V> Copy for PersistentRadixLeaf<'_, V> {}

impl<V> Clone for PersistentRadixLeaf<'_, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, V> PersistentRadixLeaf<'a, V> {
    pub(crate) fn entries(self) -> impl ExactSizeIterator<Item = (u128, &'a V)> {
        self.entries.iter().map(|entry| (entry.key, entry.value()))
    }

    pub(crate) const fn len(self) -> usize {
        self.entries.len()
    }
}

struct RadixFrame<'a, V> {
    node: &'a RadixNode<V>,
    next_item: usize,
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
            next_item: 0,
        });
        Self { frames, reverse }
    }

    fn next(&mut self) -> Option<(u128, &'a V)> {
        loop {
            let frame = self.frames.last_mut()?;
            match &frame.node.kind {
                RadixNodeKind::Leaf(entries) => {
                    if frame.next_item == entries.len() {
                        self.frames.pop();
                        continue;
                    }
                    let index = if self.reverse {
                        entries.len() - frame.next_item - 1
                    } else {
                        frame.next_item
                    };
                    frame.next_item += 1;
                    let entry = &entries[index];
                    return Some((entry.key, entry.value()));
                }
                RadixNodeKind::Branch { children, .. } => {
                    if frame.next_item == children.len() {
                        self.frames.pop();
                        continue;
                    }
                    let index = if self.reverse {
                        children.len() - frame.next_item - 1
                    } else {
                        frame.next_item
                    };
                    frame.next_item += 1;
                    self.frames.push(RadixFrame {
                        node: children[index].1.as_ref(),
                        next_item: 0,
                    });
                }
            }
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

/// A canonical adaptive radix map for immutable snapshot storage.
///
/// Branches record the first differing big-endian key byte, skipping common-prefix bytes. A
/// subtree is represented by one packed leaf whenever its entry and owned-byte bounds permit it;
/// otherwise it is partitioned at that differing byte.
/// Full builds move each leaf's values into one immutable block. Point mutations copy only block
/// handles in one bounded leaf, so unchanged payloads remain shared without per-value allocation.
#[derive(Debug)]
pub(crate) struct PersistentRadixMap<V> {
    root: Arc<RadixNode<V>>,
    len: usize,
}

impl<V> Clone for PersistentRadixMap<V> {
    fn clone(&self) -> Self {
        Self {
            root: Arc::clone(&self.root),
            len: self.len,
        }
    }
}

impl<V> Default for PersistentRadixMap<V> {
    fn default() -> Self {
        Self {
            root: Arc::new(RadixNode::leaf(Vec::new())),
            len: 0,
        }
    }
}

impl<V> PersistentRadixMap<V> {
    #[cfg(test)]
    pub(crate) fn from_sorted_iter(entries: impl IntoIterator<Item = (u128, V)>) -> Self {
        Self::from_sorted_iter_cancellable(entries, &|| false)
            .expect("non-cancellable radix construction cannot be cancelled")
    }

    pub(crate) fn from_sorted_iter_cancellable(
        entries: impl IntoIterator<Item = (u128, V)>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let iterator = entries.into_iter();
        let (lower, _) = iterator.size_hint();
        let mut sorted = Vec::with_capacity(lower);
        let mut previous = None;
        for (index, (key, value)) in iterator.enumerate() {
            if index.is_multiple_of(CANCELLATION_POLL_INTERVAL) && cancelled() {
                return Err(());
            }
            assert!(
                previous.is_none_or(|previous| previous < key),
                "radix bulk input must be strictly key-sorted"
            );
            previous = Some(key);
            sorted.push((key, value));
        }
        if cancelled() {
            return Err(());
        }
        let len = sorted.len();
        let keys = sorted.iter().map(|(key, _)| *key).collect::<Vec<_>>();
        let mut values = sorted.into_iter();
        let root = build_owned_node(&keys, &mut values, 0, leaf_capacity::<V>(), cancelled)?;
        debug_assert!(values.next().is_none());
        Ok(Self { root, len })
    }

    pub(crate) fn get(&self, key: u128) -> Option<&V> {
        get_node(self.root.as_ref(), key)
    }

    /// Inserts a value and returns the previous shared value.
    pub(crate) fn insert(&mut self, key: u128, value: V) -> Option<PersistentValue<V>> {
        let (root, previous) = insert_node(&self.root, key, value, 0, leaf_capacity::<V>());
        self.root = root;
        if previous.is_none() {
            self.len += 1;
        }
        previous
    }

    /// Removes a value and returns its shared handle.
    pub(crate) fn remove(&mut self, key: u128) -> Option<PersistentValue<V>> {
        let (root, removed) = remove_node(&self.root, key, leaf_capacity::<V>());
        if let Some(root) = root {
            self.root = root;
        } else {
            self.root = Arc::new(RadixNode::leaf(Vec::new()));
        }
        if removed.is_some() {
            self.len -= 1;
        }
        removed
    }

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
        leaf_fingerprint: &impl Fn(PersistentRadixLeaf<'_, V>) -> Result<[u8; 32], ()>,
        internal_fingerprint: &impl Fn(usize, &[(u8, [u8; 32])]) -> [u8; 32],
        cancelled: &impl Fn() -> bool,
    ) -> Result<[u8; 32], ()> {
        fingerprint_node(
            self.root.as_ref(),
            leaf_fingerprint,
            internal_fingerprint,
            cancelled,
        )
    }
}

fn leaf_capacity<V>() -> usize {
    let owned_bytes = (size_of::<LeafEntry<V>>() + size_of::<V>()).max(1);
    LEAF_ENTRY_CAPACITY.min((LEAF_OWNED_BYTE_CAPACITY / owned_bytes).max(1))
}

fn key_byte(key: u128, depth: usize) -> u8 {
    debug_assert!(depth < KEY_BYTES);
    ((key >> ((KEY_BYTES - depth - 1) * u8::BITS as usize)) & u128::from(u8::MAX)) as u8
}

fn first_differing_byte(left: u128, right: u128, start: usize) -> Option<usize> {
    (start..KEY_BYTES).find(|depth| key_byte(left, *depth) != key_byte(right, *depth))
}

fn get_node<V>(node: &RadixNode<V>, key: u128) -> Option<&V> {
    match &node.kind {
        RadixNodeKind::Leaf(entries) => entries
            .binary_search_by_key(&key, |entry| entry.key)
            .ok()
            .map(|index| entries[index].value()),
        RadixNodeKind::Branch { depth, children } => {
            let edge = key_byte(key, usize::from(*depth));
            let index = children
                .binary_search_by_key(&edge, |(candidate, _)| *candidate)
                .ok()?;
            get_node(children[index].1.as_ref(), key)
        }
    }
}

fn build_owned_node<V>(
    keys: &[u128],
    values: &mut std::vec::IntoIter<(u128, V)>,
    depth: usize,
    capacity: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Arc<RadixNode<V>>, ()> {
    if cancelled() {
        return Err(());
    }
    if keys.len() <= capacity {
        let mut owned = Vec::with_capacity(keys.len());
        for expected_key in keys {
            let (key, value) = values.next().expect("one radix value per sorted key");
            debug_assert_eq!(*expected_key, key);
            owned.push(value);
        }
        let values = Arc::<[V]>::from(owned);
        let entries = keys
            .iter()
            .enumerate()
            .map(|(index, key)| LeafEntry {
                key: *key,
                values: Arc::clone(&values),
                index,
            })
            .collect();
        return Ok(Arc::new(RadixNode::leaf(entries)));
    }
    let branch_depth = first_differing_byte(keys[0], keys[keys.len() - 1], depth)
        .expect("duplicate radix keys exceeded leaf capacity");
    let mut children = Vec::new();
    let mut start = 0;
    while start < keys.len() {
        let edge = key_byte(keys[start], branch_depth);
        let mut end = start + 1;
        while end < keys.len() && key_byte(keys[end], branch_depth) == edge {
            if (end - start).is_multiple_of(CANCELLATION_POLL_INTERVAL) && cancelled() {
                return Err(());
            }
            end += 1;
        }
        children.push((
            edge,
            build_owned_node(
                &keys[start..end],
                values,
                branch_depth + 1,
                capacity,
                cancelled,
            )?,
        ));
        start = end;
    }
    Ok(Arc::new(RadixNode::branch(
        branch_depth,
        children,
        keys.len(),
    )))
}

fn build_shared_node<V>(
    entries: &[LeafEntry<V>],
    depth: usize,
    capacity: usize,
) -> Arc<RadixNode<V>> {
    if entries.len() <= capacity {
        return Arc::new(RadixNode::leaf(entries.to_vec()));
    }
    let branch_depth = first_differing_byte(entries[0].key, entries[entries.len() - 1].key, depth)
        .expect("duplicate radix keys exceeded leaf capacity");
    let mut children = Vec::new();
    let mut start = 0;
    while start < entries.len() {
        let edge = key_byte(entries[start].key, branch_depth);
        let mut end = start + 1;
        while end < entries.len() && key_byte(entries[end].key, branch_depth) == edge {
            end += 1;
        }
        children.push((
            edge,
            build_shared_node(&entries[start..end], branch_depth + 1, capacity),
        ));
        start = end;
    }
    Arc::new(RadixNode::branch(branch_depth, children, entries.len()))
}

fn insert_node<V>(
    node: &Arc<RadixNode<V>>,
    key: u128,
    value: V,
    depth: usize,
    capacity: usize,
) -> (Arc<RadixNode<V>>, Option<PersistentValue<V>>) {
    match &node.kind {
        RadixNodeKind::Leaf(current) => {
            let mut entries = current.clone();
            let previous = match entries.binary_search_by_key(&key, |entry| entry.key) {
                Ok(index) => {
                    let previous = entries[index].shared_value();
                    entries[index] = LeafEntry::singleton(key, value);
                    Some(previous)
                }
                Err(index) => {
                    entries.insert(index, LeafEntry::singleton(key, value));
                    None
                }
            };
            let replacement = if entries.len() <= capacity {
                Arc::new(RadixNode::leaf(entries))
            } else {
                build_shared_node(&entries, depth, capacity)
            };
            (replacement, previous)
        }
        RadixNodeKind::Branch {
            depth: current_depth,
            children: current,
        } => {
            let current_depth = usize::from(*current_depth);
            if let Some(split_depth) = first_differing_byte(key, node.min_key, depth)
                && split_depth < current_depth
            {
                let new_edge = key_byte(key, split_depth);
                let existing_edge = key_byte(node.min_key, split_depth);
                let new_leaf = Arc::new(RadixNode::leaf(vec![LeafEntry::singleton(key, value)]));
                let mut children = vec![(existing_edge, Arc::clone(node)), (new_edge, new_leaf)];
                children.sort_unstable_by_key(|(edge, _)| *edge);
                return (
                    Arc::new(RadixNode::branch(split_depth, children, node.len + 1)),
                    None,
                );
            }
            let edge = key_byte(key, current_depth);
            let mut children = current.clone();
            let previous = match children.binary_search_by_key(&edge, |(candidate, _)| *candidate) {
                Ok(index) => {
                    let (child, previous) =
                        insert_node(&children[index].1, key, value, current_depth + 1, capacity);
                    children[index].1 = child;
                    previous
                }
                Err(index) => {
                    children.insert(
                        index,
                        (
                            edge,
                            Arc::new(RadixNode::leaf(vec![LeafEntry::singleton(key, value)])),
                        ),
                    );
                    None
                }
            };
            (
                Arc::new(RadixNode::branch(
                    current_depth,
                    children,
                    node.len + usize::from(previous.is_none()),
                )),
                previous,
            )
        }
    }
}

fn remove_node<V>(
    node: &Arc<RadixNode<V>>,
    key: u128,
    capacity: usize,
) -> (Option<Arc<RadixNode<V>>>, Option<PersistentValue<V>>) {
    match &node.kind {
        RadixNodeKind::Leaf(current) => {
            let Ok(index) = current.binary_search_by_key(&key, |entry| entry.key) else {
                return (Some(Arc::clone(node)), None);
            };
            let mut entries = current.clone();
            let removed = entries[index].shared_value();
            entries.remove(index);
            if entries.is_empty() {
                (None, Some(removed))
            } else {
                (Some(Arc::new(RadixNode::leaf(entries))), Some(removed))
            }
        }
        RadixNodeKind::Branch {
            depth: current_depth,
            children: current,
        } => {
            let current_depth = usize::from(*current_depth);
            let edge = key_byte(key, current_depth);
            let Ok(index) = current.binary_search_by_key(&edge, |(candidate, _)| *candidate) else {
                return (Some(Arc::clone(node)), None);
            };
            let (child, removed) = remove_node(&current[index].1, key, capacity);
            if removed.is_none() {
                return (Some(Arc::clone(node)), None);
            }
            let mut children = current.clone();
            match child {
                Some(child) => children[index].1 = child,
                None => {
                    children.remove(index);
                }
            }
            let len = node.len - 1;
            if len <= capacity {
                let mut entries = Vec::with_capacity(len);
                collect_entries(&children, &mut entries);
                (Some(Arc::new(RadixNode::leaf(entries))), removed)
            } else if children.len() == 1 {
                (Some(Arc::clone(&children[0].1)), removed)
            } else {
                (
                    Some(Arc::new(RadixNode::branch(current_depth, children, len))),
                    removed,
                )
            }
        }
    }
}

fn collect_entries<V>(children: &[(u8, Arc<RadixNode<V>>)], entries: &mut Vec<LeafEntry<V>>) {
    for (_, child) in children {
        match &child.kind {
            RadixNodeKind::Leaf(child_entries) => entries.extend_from_slice(child_entries),
            RadixNodeKind::Branch {
                children: grandchildren,
                ..
            } => collect_entries(grandchildren, entries),
        }
    }
}

fn fingerprint_node<V>(
    node: &RadixNode<V>,
    leaf_fingerprint: &impl Fn(PersistentRadixLeaf<'_, V>) -> Result<[u8; 32], ()>,
    internal_fingerprint: &impl Fn(usize, &[(u8, [u8; 32])]) -> [u8; 32],
    cancelled: &impl Fn() -> bool,
) -> Result<[u8; 32], ()> {
    if let Some(fingerprint) = node.semantic_fingerprint.get() {
        return Ok(*fingerprint);
    }
    if cancelled() {
        return Err(());
    }
    let fingerprint = match &node.kind {
        RadixNodeKind::Leaf(entries) => leaf_fingerprint(PersistentRadixLeaf { entries })?,
        RadixNodeKind::Branch { depth, children } => {
            let mut child_fingerprints = Vec::with_capacity(children.len());
            for (edge, child) in children {
                if cancelled() {
                    return Err(());
                }
                child_fingerprints.push((
                    *edge,
                    fingerprint_node(
                        child.as_ref(),
                        leaf_fingerprint,
                        internal_fingerprint,
                        cancelled,
                    )?,
                ));
            }
            internal_fingerprint(usize::from(*depth), &child_fingerprints)
        }
    };
    let _ = node.semantic_fingerprint.set(fingerprint);
    Ok(*node
        .semantic_fingerprint
        .get()
        .expect("radix fingerprint was initialized"))
}

#[cfg(test)]
mod tests {
    use super::{PersistentRadixLeaf, PersistentRadixMap};

    fn leaf(leaf: PersistentRadixLeaf<'_, u64>) -> Result<[u8; 32], ()> {
        let mut digest = [0_u8; 32];
        for (key, value) in leaf.entries() {
            for (index, byte) in key.to_be_bytes().into_iter().enumerate() {
                digest[index] ^= byte;
            }
            for (index, byte) in value.to_be_bytes().into_iter().enumerate() {
                digest[index + 16] ^= byte;
            }
        }
        Ok(digest)
    }

    fn internal(depth: usize, children: &[(u8, [u8; 32])]) -> [u8; 32] {
        let mut output = [0_u8; 32];
        output[0] = depth as u8;
        for (edge, digest) in children {
            output[1] ^= *edge;
            for (target, source) in output[2..].iter_mut().zip(digest) {
                *target ^= *source;
            }
        }
        output
    }

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
        assert_eq!(
            ascending.semantic_fingerprint_cancellable(&leaf, &internal, &|| false),
            descending.semantic_fingerprint_cancellable(&leaf, &internal, &|| false)
        );
    }

    #[test]
    fn bulk_build_matches_incremental_shape_and_remove_merges_canonically() {
        let entries = (0_u64..600)
            .map(|value| (u128::from(value) << 8, value))
            .collect::<Vec<_>>();
        let bulk = PersistentRadixMap::from_sorted_iter(entries.iter().copied());
        let mut incremental = PersistentRadixMap::default();
        for (key, value) in entries.iter().rev().copied() {
            incremental.insert(key, value);
        }
        assert_eq!(
            bulk.semantic_fingerprint_cancellable(&leaf, &internal, &|| false),
            incremental.semantic_fingerprint_cancellable(&leaf, &internal, &|| false)
        );

        for (key, _) in entries.iter().skip(200) {
            assert!(incremental.remove(*key).is_some());
        }
        let expected = PersistentRadixMap::from_sorted_iter(entries[..200].iter().copied());
        assert_eq!(
            expected.semantic_fingerprint_cancellable(&leaf, &internal, &|| false),
            incremental.semantic_fingerprint_cancellable(&leaf, &internal, &|| false)
        );
    }

    #[test]
    fn streaming_iterator_supports_mixed_front_and_back_consumption() {
        let map = PersistentRadixMap::from_sorted_iter([
            (1_u128, 1_u128),
            (2, 2),
            (257, 257),
            (u128::MAX, u128::MAX),
        ]);
        let mut entries = map.ordered_entries();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries.next().map(|(key, _)| key), Some(1));
        assert_eq!(entries.next_back().map(|(key, _)| key), Some(u128::MAX));
        assert_eq!(entries.next().map(|(key, _)| key), Some(2));
        assert_eq!(entries.next_back().map(|(key, _)| key), Some(257));
        assert_eq!(entries.next(), None);
        assert_eq!(entries.next_back(), None);
    }

    #[test]
    fn bulk_build_observes_cancellation() {
        assert!(
            PersistentRadixMap::from_sorted_iter_cancellable(
                (0_u128..1_000).map(|key| (key, key)),
                &|| true,
            )
            .is_err()
        );
    }
}
