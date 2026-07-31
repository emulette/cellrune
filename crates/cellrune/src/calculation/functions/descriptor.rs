use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::sheet_span::SheetSpanPolicy;
use super::super::value::{ErrorKind, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum FunctionId {
    Areas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FunctionDescriptor {
    id: FunctionId,
    canonical_name: &'static str,
    returns_array: bool,
    returns_reference: bool,
    sheet_span_policy: SheetSpanPolicy,
    reference_metadata_only: bool,
    official: bool,
}

impl FunctionDescriptor {
    const fn new(
        id: FunctionId,
        canonical_name: &'static str,
        returns_array: bool,
        returns_reference: bool,
        sheet_span_policy: SheetSpanPolicy,
        reference_metadata_only: bool,
        official: bool,
    ) -> Self {
        Self {
            id,
            canonical_name,
            returns_array,
            returns_reference,
            sheet_span_policy,
            reference_metadata_only,
            official,
        }
    }

    pub(super) const fn canonical_name(self) -> &'static str {
        self.canonical_name
    }

    pub(super) const fn returns_array(self) -> bool {
        self.returns_array
    }

    pub(super) const fn returns_reference(self) -> bool {
        self.returns_reference
    }

    pub(super) const fn sheet_span_policy(self) -> SheetSpanPolicy {
        self.sheet_span_policy
    }

    pub(super) const fn reference_metadata_only(self) -> bool {
        self.reference_metadata_only
    }

    pub(super) const fn is_official(self) -> bool {
        self.official
    }
}

const DESCRIPTORS: &[FunctionDescriptor] = &[FunctionDescriptor::new(
    FunctionId::Areas,
    "AREAS",
    false,
    false,
    SheetSpanPolicy::ReturnExcelError(ErrorKind::Value),
    true,
    true,
)];

pub(super) fn descriptors() -> &'static [FunctionDescriptor] {
    DESCRIPTORS
}

pub(super) fn descriptor(canonical_name: &str) -> Option<FunctionDescriptor> {
    DESCRIPTORS
        .iter()
        .copied()
        .find(|candidate| candidate.canonical_name == canonical_name)
}

pub(super) fn dispatch(
    descriptor: FunctionDescriptor,
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    match descriptor.id {
        FunctionId::Areas => areas(engine, context, args),
    }
}

fn areas(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [reference] = args else {
        return Value::Error(ErrorKind::Value);
    };
    match engine.resolve_reference_value_expr(context, reference) {
        Ok(crate::calculation::runtime::ReferenceValue::Empty) => Value::Error(ErrorKind::Ref),
        Ok(reference) if reference.has_sheet_span() => Value::Error(ErrorKind::Value),
        Ok(reference) => Value::Number(reference.area_count() as f64),
        Err(kind) => Value::Error(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_lookup_exposes_stable_areas_identity_and_flags() {
        let areas = descriptor("AREAS").expect("AREAS descriptor");

        assert_eq!(areas.id, FunctionId::Areas);
        assert_eq!(areas.canonical_name(), "AREAS");
        assert!(!areas.returns_array());
        assert!(!areas.returns_reference());
        assert_eq!(
            areas.sheet_span_policy(),
            SheetSpanPolicy::ReturnExcelError(ErrorKind::Value)
        );
        assert!(areas.reference_metadata_only());
        assert!(areas.is_official());
        assert!(descriptor("areas").is_none());
    }
}
