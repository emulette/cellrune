//! Classify name aliases without adding one host stack frame per definition.

use std::collections::{BTreeMap, BTreeSet};

use super::Engine;
use crate::DefinedNameScope;
use crate::calculation::error::MESSAGE_CALLABLE_CLASSIFICATION_FRAME;
use crate::calculation::functions::{CallableShadow, classify_callable_value};
use crate::calculation::scope::DefinedLambdaId;

struct Frame {
    index: usize,
    id: DefinedLambdaId,
    resolved: BTreeMap<DefinedLambdaId, CallableShadow>,
    cacheable: bool,
}

impl Frame {
    fn new(index: usize, id: DefinedLambdaId) -> Self {
        Self {
            index,
            id,
            resolved: BTreeMap::new(),
            cacheable: true,
        }
    }
}

impl Engine<'_> {
    pub(in crate::calculation) fn callable_shadow_for_name(
        &self,
        sheet: usize,
        lookup_scope: Option<DefinedNameScope>,
        name: &str,
    ) -> CallableShadow {
        let Some((index, definition)) = self.resolve_defined_name_scoped(sheet, lookup_scope, name)
        else {
            return CallableShadow::Unshadowed;
        };
        let id = DefinedLambdaId::from_defined_name(definition);
        let mut active = BTreeSet::from([id.clone()]);
        let mut frames = vec![Frame::new(index, id)];
        // The caller sheet is fixed for this classification. Definition identity includes scope.
        let mut memo = BTreeMap::new();
        loop {
            let frame = frames
                .last_mut()
                .expect(MESSAGE_CALLABLE_CLASSIFICATION_FRAME);
            let mut cut_cycle = false;
            let classification = self
                .defined_name_asts
                .get(frame.index)
                .and_then(Option::as_ref)
                .map_or(Ok(CallableShadow::Unknown), |parsed| {
                    let mut resolve = |nested: &str| {
                        let Some((index, definition)) =
                            self.resolve_defined_name_scoped(sheet, Some(frame.id.scope()), nested)
                        else {
                            return Ok(CallableShadow::Unshadowed);
                        };
                        let id = DefinedLambdaId::from_defined_name(definition);
                        if active.contains(&id) {
                            cut_cycle = true;
                            return Ok(CallableShadow::CyclicNonCallable);
                        }
                        if let Some(state) = frame.resolved.get(&id).or_else(|| memo.get(&id)) {
                            return Ok(*state);
                        }
                        // Suspend at the unresolved definition and resume this AST after its
                        // child completes. Existing AST classification owns LET/local semantics.
                        Err((index, id))
                    };
                    classify_callable_value(
                        parsed.root(),
                        &[],
                        self.calculation_limits().max_let_bindings(),
                        &mut resolve,
                    )
                });
            if cut_cycle {
                for frame in &mut frames {
                    frame.cacheable = false;
                }
            }
            match classification {
                Err((index, id)) => {
                    active.insert(id.clone());
                    frames.push(Frame::new(index, id));
                }
                Ok(state) => {
                    let frame = frames.pop().expect(MESSAGE_CALLABLE_CLASSIFICATION_FRAME);
                    active.remove(&frame.id);
                    if frame.cacheable {
                        memo.insert(frame.id.clone(), state);
                    }
                    let Some(parent) = frames.last_mut() else {
                        return state;
                    };
                    // Even a cycle-dependent child is reusable by this same suspended parent,
                    // whose active ancestors have not changed; it never enters the shared memo.
                    parent.resolved.insert(frame.id, state);
                }
            }
        }
    }
}
