//! Deterministic draws for `RAND`, `RANDBETWEEN`, and `RANDARRAY`.
//!
//! A draw never depends on calculation order. Its key folds in the seed, the formula cell's stable
//! sheet ID and address, and a [`RandomPath`]: the structural path evaluation took from the cell's
//! formula to the call (argument and operand positions, defined-name references, LAMBDA calls)
//! with the LAMBDA iterations, aggregated groups, and database records on the way. The element counter then indexes a SplitMix64 sequence
//! from that key. This is a small non-cryptographic mixer, not a statistical-grade generator.

use super::ast::Expr;

const GOLDEN_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
const SEED_DOMAIN: u64 = 0x5241_4e44_5345_4544;

const CHILD_TAG: u64 = 0;
const ITERATION_TAG: u64 = 1;
const LAMBDA_BODY_TAG: u64 = 2;
const GROUP_TAG: u64 = 3;

/// The SplitMix64 output function, a bijection on `u64`.
const fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

const fn absorb(state: u64, word: u64) -> u64 {
    mix(state.wrapping_add(GOLDEN_GAMMA) ^ word)
}

/// How evaluation reached the current expression, folded into one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RandomPath {
    state: u64,
    children: Children,
}

/// The direct operands of the expression evaluation last entered, identified by address only.
///
/// Function kernels evaluate their arguments themselves, so the path learns an argument's position
/// when evaluation enters an expression that sits in this list. Addresses are compared, never
/// dereferenced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Children {
    None,
    Slice { start: usize, len: usize },
    Pair { left: usize, right: usize },
}

impl Children {
    fn of(expr: &Expr) -> Self {
        match expr {
            Expr::Call { args, .. } | Expr::Invoke { args, .. } => Self::slice(args),
            Expr::Binary { left, right, .. }
            | Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right }
            | Expr::Range {
                start: left,
                end: right,
            } => Self::Pair {
                left: address(left),
                right: address(right),
            },
            _ => Self::None,
        }
    }

    fn slice(args: &[Expr]) -> Self {
        Self::Slice {
            start: args.as_ptr() as usize,
            len: args.len(),
        }
    }

    fn position(self, expr: &Expr) -> Option<u64> {
        let expr = address(expr);
        match self {
            Self::None => None,
            Self::Slice { start, len } => {
                let offset = expr.checked_sub(start)?;
                let size = size_of::<Expr>();
                let index = offset / size;
                (offset % size == 0 && index < len).then_some(index as u64)
            }
            Self::Pair { left, right } => {
                if expr == left {
                    Some(0)
                } else if expr == right {
                    Some(1)
                } else {
                    None
                }
            }
        }
    }
}

fn address(expr: &Expr) -> usize {
    std::ptr::from_ref(expr) as usize
}

impl RandomPath {
    pub(super) const ROOT: Self = Self {
        state: 0,
        children: Children::None,
    };

    /// Steps into `expr`, recording its position when it is an operand of the expression entered
    /// before it. Entering the same expression again leaves the path unchanged.
    pub(super) fn enter(self, expr: &Expr) -> Self {
        let state = match self.children.position(expr) {
            Some(index) => absorb(self.state, (index << 2) | CHILD_TAG),
            None => self.state,
        };
        Self {
            state,
            children: Children::of(expr),
        }
    }

    /// Replaces the operands with the prepared argument list a kernel will evaluate.
    ///
    /// Default arguments are materialized into a copy, so the copy's positions stand in for the
    /// call's own arguments.
    pub(super) fn with_arguments(self, args: &[Expr]) -> Self {
        Self {
            state: self.state,
            children: Children::slice(args),
        }
    }

    /// Records the index at which a LAMBDA helper calls its function.
    pub(super) fn iteration(self, index: u64) -> Self {
        Self {
            state: absorb(self.state, (index << 2) | ITERATION_TAG),
            children: Children::None,
        }
    }

    /// Records a grouped aggregation call by its value column and the group's own member rows, so
    /// every call that aggregates the same group draws alike whatever order it is made in and
    /// whatever rows it is compared against.
    pub(super) fn group(self, column: u32, members: &[u32]) -> Self {
        let group = absorb(self.state, (u64::from(column) << 2) | GROUP_TAG);
        let state = members
            .iter()
            .fold(absorb(group, members.len() as u64), |state, row| {
                absorb(state, u64::from(*row))
            });
        Self {
            state,
            children: Children::None,
        }
    }

    /// Records entry into a LAMBDA body from the call that invoked it.
    pub(super) fn lambda_body(self) -> Self {
        Self {
            state: absorb(self.state, LAMBDA_BODY_TAG),
            children: Children::None,
        }
    }

    /// Returns the draws of one call, indexed by array element.
    pub(super) fn draws(self, seed: u32, sheet_id: u32, row: u32, column: u32) -> RandomDraws {
        let key = [u64::from(sheet_id), u64::from(row), u64::from(column)]
            .into_iter()
            .fold(mix(u64::from(seed) ^ SEED_DOMAIN), absorb);
        RandomDraws {
            key: absorb(key, self.state),
        }
    }
}

/// A counter-indexed sequence of uniform draws in `[0, 1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RandomDraws {
    key: u64,
}

impl RandomDraws {
    pub(super) fn uniform(self, element: u64) -> f64 {
        let bits = mix(self
            .key
            .wrapping_add(element.wrapping_add(1).wrapping_mul(GOLDEN_GAMMA)));
        (bits >> 11) as f64 * (1.0 / (1_u64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(value: f64) -> Expr {
        Expr::number(value)
    }

    #[test]
    fn operands_are_recognized_by_position_once() {
        let args = vec![number(1.0), number(2.0), number(3.0)];
        let call = Expr::Call {
            name: "SUM".to_owned(),
            args,
        };
        let Expr::Call { args, .. } = &call else {
            unreachable!("constructed as a call");
        };
        let entered = RandomPath::ROOT.enter(&call);
        assert_eq!(entered, RandomPath::ROOT.enter(&call).enter(&call));
        let first = entered.enter(&args[0]);
        let second = entered.enter(&args[1]);
        assert_ne!(first.state, second.state);
        assert_eq!(first.state, first.enter(&args[0]).state);
        let copy = args.clone();
        assert_eq!(
            entered.with_arguments(&copy).enter(&copy[1]).state,
            second.state
        );
        assert_eq!(entered.enter(&number(2.0)).state, entered.state);
    }

    #[test]
    fn tags_keep_positions_iterations_and_bodies_apart() {
        let args = vec![number(1.0), number(2.0)];
        let call = Expr::Call {
            name: "SUM".to_owned(),
            args,
        };
        let Expr::Call { args, .. } = &call else {
            unreachable!("constructed as a call");
        };
        let entered = RandomPath::ROOT.enter(&call);
        let states = [
            entered.enter(&args[1]).state,
            entered.iteration(1).state,
            entered.lambda_body().state,
            entered.iteration(0).state,
            entered.group(1, &[]).state,
            entered.group(1, &[1]).state,
            entered.group(2, &[1]).state,
        ];
        for (index, state) in states.iter().enumerate() {
            assert!(!states[index + 1..].contains(state));
        }
    }

    #[test]
    fn draws_are_keyed_by_seed_cell_and_element() {
        let path = RandomPath::ROOT;
        let base = path.draws(7, 1, 1, 1);
        assert_eq!(base.uniform(0), path.draws(7, 1, 1, 1).uniform(0));
        let variants = [
            base.uniform(1),
            path.draws(8, 1, 1, 1).uniform(0),
            path.draws(7, 2, 1, 1).uniform(0),
            path.draws(7, 1, 2, 1).uniform(0),
            path.draws(7, 1, 1, 2).uniform(0),
            path.lambda_body().draws(7, 1, 1, 1).uniform(0),
        ];
        assert!(variants.iter().all(|value| *value != base.uniform(0)));
        let draws = (0..100_000).map(|element| base.uniform(element));
        let (minimum, maximum, sum) = draws.fold((1.0, 0.0, 0.0), |(low, high, sum), value| {
            assert!((0.0..1.0).contains(&value));
            (f64::min(low, value), f64::max(high, value), sum + value)
        });
        assert!(minimum < 0.001 && maximum > 0.999);
        assert!((sum / 100_000.0 - 0.5).abs() < 0.01);
    }
}
