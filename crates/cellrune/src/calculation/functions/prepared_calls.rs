//! The prepared argument lists of a formula's calls, so evaluating a call whose omitted arguments
//! take defaults reads the list its formula keeps instead of copying the arguments each time.

use std::borrow::Cow;

use super::super::ast::Expr;
use super::super::prepared_arguments::PreparedArgumentTable;
use super::{CallArguments, call_shape_is_valid, descriptor};

/// Returns the prepared lists of the calls under `root` whose arguments take defaults. Evaluation
/// reads a prepared list in place of the call's own, so the calls inside it are taken from the
/// prepared list rather than the written one.
pub(in crate::calculation) fn prepared_argument_table(root: &Expr) -> PreparedArgumentTable {
    let mut table = PreparedArgumentTable::new();
    collect(root, &mut table);
    table
}

fn collect(expr: &Expr, table: &mut PreparedArgumentTable) {
    crate::calculation::stack::grow(|| match expr {
        Expr::Call { name, args } => {
            // Every empty list has the same address, so an empty one is prepared when called.
            if !args.is_empty()
                && let Some(descriptor) = descriptor::resolve(name)
                && call_shape_is_valid(descriptor, args)
                && let Cow::Owned(prepared) = CallArguments::prepare(descriptor, args).expressions
            {
                let prepared = prepared.into_boxed_slice();
                for arg in &prepared {
                    collect(arg, table);
                }
                table.insert((args.as_ptr() as usize, args.len()), prepared);
                return;
            }
            for arg in args {
                collect(arg, table);
            }
        }
        Expr::Invoke { callee, args } => {
            collect(callee, table);
            for arg in args {
                collect(arg, table);
            }
        }
        Expr::ReferenceUnion { left, right }
        | Expr::ReferenceIntersection { left, right }
        | Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
        } => {
            collect(left, table);
            collect(right, table);
        }
        Expr::SpillRef(inner)
        | Expr::ImplicitIntersection(inner)
        | Expr::Paren(inner)
        | Expr::Unary { operand: inner, .. } => collect(inner, table),
        Expr::Array(rows) => {
            for item in rows.iter().flatten() {
                collect(item, table);
            }
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::Ref(_)
        | Expr::StructuredRef(_)
        | Expr::ExternalReference(_)
        | Expr::QualifiedName { .. }
        | Expr::Name(_)
        | Expr::BuiltinCallable(_)
        | Expr::Missing => {}
    });
}
