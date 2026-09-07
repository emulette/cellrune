use std::cmp::Ordering;

use super::{
    Arc, CANCELLATION_POLL_INTERVAL, LeafEntry, PersistentRadixMap, RadixNode, RadixNodeKind,
    collect_entries, first_differing_byte, key_byte, leaf_capacity,
};

impl<V> PersistentRadixMap<V> {
    /// Applies strictly sorted upserts and removals, rebuilding each affected subtree once.
    /// Cancellation leaves this map and all snapshots sharing it unchanged.
    pub(crate) fn apply_sorted_patch_cancellable(
        &mut self,
        changes: impl IntoIterator<Item = (u128, Option<V>)>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<usize, ()> {
        let changes = changes.into_iter();
        let mut patch = Vec::with_capacity(changes.size_hint().0);
        let mut previous = None;
        for (index, (key, value)) in changes.enumerate() {
            if index.is_multiple_of(CANCELLATION_POLL_INTERVAL) && cancelled() {
                return Err(());
            }
            debug_assert!(previous.is_none_or(|previous| previous < key));
            previous = Some(key);
            patch.push((key, value));
        }
        let root = patch_node(Some(&self.root), &mut patch, 0, cancelled)?;
        if cancelled() {
            return Err(());
        }
        self.root = root.unwrap_or_else(|| Arc::new(RadixNode::leaf(Vec::new())));
        self.len = self.root.len;
        Ok(self.len)
    }
}

fn patch_node<V>(
    node: Option<&Arc<RadixNode<V>>>,
    patch: &mut [(u128, Option<V>)],
    depth: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Arc<RadixNode<V>>>, ()> {
    if cancelled() {
        return Err(());
    }
    if patch.is_empty() {
        return Ok(node.cloned());
    }
    let Some(node) = node else {
        return patch_leaf(&[], patch, depth, cancelled);
    };
    let (current_depth, children) = match &node.kind {
        RadixNodeKind::Leaf(entries) => return patch_leaf(entries, patch, depth, cancelled),
        RadixNodeKind::Branch { depth, children } => (usize::from(*depth), children),
    };
    let branch_depth = [patch[0].0, patch[patch.len() - 1].0]
        .into_iter()
        .filter_map(|key| first_differing_byte(key, node.min_key, depth))
        .fold(current_depth, usize::min);
    let wrapped;
    let children = if branch_depth < current_depth {
        wrapped = [(key_byte(node.min_key, branch_depth), Arc::clone(node))];
        wrapped.as_slice()
    } else {
        children.as_slice()
    };
    let mut current = children.iter().peekable();
    let mut output = Vec::with_capacity(children.len());
    let mut remaining = patch;
    while !remaining.is_empty() {
        let edge = key_byte(remaining[0].0, branch_depth);
        while current.peek().is_some_and(|(old_edge, _)| *old_edge < edge) {
            let (old_edge, child) = current.next().expect("peeked radix child");
            output.push((*old_edge, Arc::clone(child)));
        }
        let child = if current
            .peek()
            .is_some_and(|(old_edge, _)| *old_edge == edge)
        {
            current.next().map(|(_, child)| child)
        } else {
            None
        };
        let end = remaining.partition_point(|(key, _)| key_byte(*key, branch_depth) == edge);
        let (group, rest) = remaining.split_at_mut(end);
        if let Some(child) = patch_node(child, group, branch_depth + 1, cancelled)? {
            output.push((edge, child));
        }
        remaining = rest;
    }
    output.extend(current.map(|(edge, child)| (*edge, Arc::clone(child))));
    let len = output.iter().map(|(_, child)| child.len).sum();
    if len == 0 {
        Ok(None)
    } else if output.len() == 1 {
        Ok(output.pop().map(|(_, child)| child))
    } else if len <= leaf_capacity::<V>() {
        let mut entries = Vec::with_capacity(len);
        collect_entries(&output, &mut entries);
        Ok(Some(Arc::new(RadixNode::leaf(entries))))
    } else {
        Ok(Some(Arc::new(RadixNode::branch(branch_depth, output, len))))
    }
}

enum MergedEntry<V> {
    Shared(LeafEntry<V>),
    Owned(u128, V),
}

impl<V> MergedEntry<V> {
    fn key(&self) -> u128 {
        match self {
            Self::Shared(entry) => entry.key,
            Self::Owned(key, _) => *key,
        }
    }
}

fn patch_leaf<V>(
    entries: &[LeafEntry<V>],
    patch: &mut [(u128, Option<V>)],
    depth: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Arc<RadixNode<V>>>, ()> {
    let mut current = entries.iter().peekable();
    let mut merged = Vec::with_capacity(entries.len() + patch.len());
    for (index, (key, value)) in patch.iter_mut().enumerate() {
        if index.is_multiple_of(CANCELLATION_POLL_INTERVAL) && cancelled() {
            return Err(());
        }
        while let Some(entry) = current.peek() {
            match entry.key.cmp(key) {
                Ordering::Less => {
                    merged.push(MergedEntry::Shared((*entry).clone()));
                    current.next();
                }
                Ordering::Equal => {
                    current.next();
                    break;
                }
                Ordering::Greater => break,
            }
        }
        if let Some(value) = value.take() {
            merged.push(MergedEntry::Owned(*key, value));
        }
    }
    merged.extend(current.cloned().map(MergedEntry::Shared));
    if merged.is_empty() {
        return Ok(None);
    }
    let mut keys = Vec::with_capacity(merged.len());
    for (index, entry) in merged.iter().enumerate() {
        if index.is_multiple_of(CANCELLATION_POLL_INTERVAL) && cancelled() {
            return Err(());
        }
        keys.push(entry.key());
    }
    build_merged_node(&keys, &mut merged.into_iter(), depth, cancelled).map(Some)
}

fn build_merged_node<V>(
    keys: &[u128],
    entries: &mut std::vec::IntoIter<MergedEntry<V>>,
    depth: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Arc<RadixNode<V>>, ()> {
    if cancelled() {
        return Err(());
    }
    if keys.len() <= leaf_capacity::<V>() {
        let mut leaf = Vec::with_capacity(keys.len());
        for _ in keys {
            let entry = match entries.next().expect("one merged radix entry per key") {
                MergedEntry::Shared(entry) => entry,
                // A surviving sibling must not retain values overwritten by a later patch.
                MergedEntry::Owned(key, value) => LeafEntry::singleton(key, value),
            };
            leaf.push(entry);
        }
        return Ok(Arc::new(RadixNode::leaf(leaf)));
    }
    let branch_depth = first_differing_byte(keys[0], keys[keys.len() - 1], depth)
        .expect("duplicate radix keys exceeded leaf capacity");
    let mut children = Vec::new();
    let mut remaining = keys;
    while !remaining.is_empty() {
        let edge = key_byte(remaining[0], branch_depth);
        let end = remaining.partition_point(|key| key_byte(*key, branch_depth) == edge);
        children.push((
            edge,
            build_merged_node(&remaining[..end], entries, branch_depth + 1, cancelled)?,
        ));
        remaining = &remaining[end..];
    }
    Ok(Arc::new(RadixNode::branch(
        branch_depth,
        children,
        keys.len(),
    )))
}
