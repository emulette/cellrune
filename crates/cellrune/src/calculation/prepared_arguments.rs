//! Kept argument lists with their defaults in place, so a call evaluated many times, as in the
//! body of a LAMBDA a helper invokes, reads one list instead of copying its arguments each time.

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::ast::Expr;

/// Argument lists with their defaults in place, by the address and length of the list they
/// prepare. The lists are an expression's own and the prepared ones', which stay where they are
/// while the expression lives, so no other list can be found at an address the table holds.
pub(super) type PreparedArgumentTable = HashMap<(usize, usize), Box<[Expr]>>;

/// The [`PreparedArgumentTable`] of an expression that does not change, built when evaluation asks
/// again for the list it asked for first, so an expression evaluated once copies each list once.
/// It caches the expression's own nodes: it compares equal to any other, and a clone, whose nodes
/// live elsewhere, starts empty.
#[derive(Default)]
pub(super) struct PreparedArguments {
    table: OnceLock<PreparedArgumentTable>,
    /// The address of the list asked for first. The first request comes before evaluation reads
    /// any prepared copy, so it is for one of the expression's own lists.
    first_request: AtomicUsize,
}

impl Clone for PreparedArguments {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl fmt::Debug for PreparedArguments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PreparedArguments")
    }
}

impl PartialEq for PreparedArguments {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// An expression being evaluated and the prepared arguments it keeps.
#[derive(Debug, Clone, Copy)]
pub(super) struct ArgumentOwner<'expr> {
    root: &'expr Expr,
    prepared: &'expr PreparedArguments,
}

impl<'expr> ArgumentOwner<'expr> {
    pub(super) const fn new(root: &'expr Expr, prepared: &'expr PreparedArguments) -> Self {
        Self { root, prepared }
    }

    /// The prepared form of `args`, when they are one of the root's argument lists and the table
    /// is built. `args` is not empty: every empty list has the same address.
    pub(super) fn prepared(self, args: &[Expr]) -> Option<&'expr [Expr]> {
        let cache = self.prepared;
        let address = args.as_ptr() as usize;
        let table = match cache.table.get() {
            Some(table) => table,
            None => match cache.first_request.compare_exchange(
                0,
                address,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Err(first) if first == address => cache
                    .table
                    .get_or_init(|| super::functions::prepared_argument_table(self.root)),
                Ok(_) | Err(_) => return None,
            },
        };
        table
            .get(&(args.as_ptr() as usize, args.len()))
            .map(|prepared| &**prepared)
    }
}

#[cfg(test)]
mod tests {
    use super::super::CalculationLimits;
    use super::super::ast::Expr;
    use super::super::parser::parse_formula_with_limits;

    fn call_args(expr: &Expr) -> &[Expr] {
        match expr {
            Expr::Call { args, .. } => args,
            _ => panic!("a call"),
        }
    }

    #[test]
    fn an_owner_keeps_the_prepared_lists_of_its_own_calls_after_the_first_request() {
        let formula = parse_formula_with_limits("IF(A1>0,IF(B1,2))", CalculationLimits::default())
            .expect("parses");
        let owner = formula.argument_owner();
        let outer = call_args(formula.root());
        assert_eq!(
            owner.prepared(outer),
            None,
            "the first request prepares a copy"
        );
        let prepared = owner.prepared(outer).expect("kept on the second request");
        assert_eq!(
            prepared,
            [outer[0].clone(), outer[1].clone(), Expr::Logical(false)]
        );
        // Evaluation reads the inner call from the outer prepared list, which the table holds too.
        let inner = owner.prepared(call_args(&prepared[1])).expect("inner call");
        assert_eq!(inner.last(), Some(&Expr::Logical(false)));
        // The written lists keep their omissions, and a clone's lists are not the formula's.
        assert_eq!(outer.len(), 2);
        let clone = formula.clone();
        assert_eq!(owner.prepared(call_args(clone.root())), None);
    }
}
