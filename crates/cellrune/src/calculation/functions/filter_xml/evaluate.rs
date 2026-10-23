//! XPath 1.0 evaluation over a [`Document`] (§2–§4).
//!
//! A text node that holds only white space is a node an XML parser that does not preserve white
//! space drops, and Excel does not document which kind it uses. Such a node may pass through a
//! path where it cannot change the result, as the `//` in `//x` passes it to a child step, but
//! a result, position, count, or string value that would see it is unsupported.

use super::document::{Document, NodeId, NodeKind, ROOT, is_xml_space};
use super::xpath::{Arithmetic, Axis, Comparison, Expr, Function, LocationPath, NodeTest, Step};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EvalError {
    /// The expression reaches behavior the evaluator does not model.
    Unsupported,
    /// The work budget refused more work.
    Budget,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum XValue {
    /// Nodes in document order without duplicates.
    Nodes(Vec<NodeId>),
    Boolean(bool),
    Number(f64),
    String(String),
}

#[derive(Debug, Clone, Copy)]
struct Context {
    node: NodeId,
    position: usize,
    size: usize,
}

pub(super) struct Evaluator<'a, F> {
    document: &'a Document,
    charge: F,
}

impl<'a, F: FnMut(u64) -> Result<(), EvalError>> Evaluator<'a, F> {
    /// Creates an evaluator that charges `charge` one unit per node visited or character read.
    pub(super) const fn new(document: &'a Document, charge: F) -> Self {
        Self { document, charge }
    }

    /// Evaluates `expr` with the root as the context node.
    pub(super) fn evaluate(&mut self, expr: &Expr) -> Result<XValue, EvalError> {
        self.eval(
            expr,
            Context {
                node: ROOT,
                position: 1,
                size: 1,
            },
        )
    }

    fn eval(&mut self, expr: &Expr, context: Context) -> Result<XValue, EvalError> {
        crate::calculation::stack::grow(|| match expr {
            Expr::Or(left, right) => Ok(XValue::Boolean(
                self.boolean(left, context)? || self.boolean(right, context)?,
            )),
            Expr::And(left, right) => Ok(XValue::Boolean(
                self.boolean(left, context)? && self.boolean(right, context)?,
            )),
            Expr::Compare(comparison, left, right) => {
                let left = self.eval(left, context)?;
                let right = self.eval(right, context)?;
                Ok(XValue::Boolean(self.compare(*comparison, &left, &right)?))
            }
            Expr::Arithmetic(operation, left, right) => {
                let left = self.number(left, context)?;
                let right = self.number(right, context)?;
                Ok(XValue::Number(match operation {
                    Arithmetic::Add => left + right,
                    Arithmetic::Subtract => left - right,
                    Arithmetic::Multiply => left * right,
                    Arithmetic::Divide => left / right,
                    // §3.5: the remainder of truncating division, with the sign of the dividend.
                    Arithmetic::Modulo => left % right,
                }))
            }
            Expr::Negate(operand) => Ok(XValue::Number(-self.number(operand, context)?)),
            Expr::Union(left, right) => {
                let (XValue::Nodes(mut left), XValue::Nodes(right)) =
                    (self.eval(left, context)?, self.eval(right, context)?)
                else {
                    return Err(EvalError::Unsupported);
                };
                left.extend(right);
                left.sort_unstable();
                left.dedup();
                Ok(XValue::Nodes(left))
            }
            Expr::Path(path) => self.location_path(path, context).map(XValue::Nodes),
            Expr::Filter {
                primary,
                predicates,
                steps,
            } => {
                let XValue::Nodes(mut nodes) = self.eval(primary, context)? else {
                    return Err(EvalError::Unsupported);
                };
                for predicate in predicates {
                    nodes = self.filter(nodes, predicate)?;
                }
                for step in steps {
                    nodes = self.step(&nodes, step)?;
                }
                Ok(XValue::Nodes(nodes))
            }
            Expr::Literal(text) => Ok(XValue::String(text.clone())),
            Expr::Number(value) => Ok(XValue::Number(*value)),
            Expr::Call(function, args) => self.call(*function, args, context),
        })
    }

    fn location_path(
        &mut self,
        path: &LocationPath,
        context: Context,
    ) -> Result<Vec<NodeId>, EvalError> {
        let mut nodes = vec![if path.absolute { ROOT } else { context.node }];
        for step in &path.steps {
            nodes = self.step(&nodes, step)?;
        }
        Ok(nodes)
    }

    fn step(&mut self, nodes: &[NodeId], step: &Step) -> Result<Vec<NodeId>, EvalError> {
        let mut selected = Vec::new();
        for node in nodes {
            if is_white_space_text(&self.document.node(*node).kind) {
                match step.axis {
                    // A text node has no children, attributes, or descendants.
                    Axis::Child | Axis::Attribute | Axis::Descendant => continue,
                    Axis::Itself | Axis::DescendantOrSelf => {}
                    _ => return Err(EvalError::Unsupported),
                }
            }
            let mut candidates = Vec::new();
            for candidate in self.axis(*node, step.axis)? {
                if matches_test(self.document, candidate, step.axis, &step.test) {
                    candidates.push(candidate);
                }
            }
            // Predicates count positions along the axis: nearest first on a reverse axis.
            for predicate in &step.predicates {
                candidates = self.filter(candidates, predicate)?;
            }
            selected.extend(candidates);
        }
        selected.sort_unstable();
        selected.dedup();
        Ok(selected)
    }

    /// Keeps the nodes of `nodes`, in their order, for which `predicate` holds: a number
    /// predicate compares with the position.
    fn filter(&mut self, nodes: Vec<NodeId>, predicate: &Expr) -> Result<Vec<NodeId>, EvalError> {
        self.observe(&nodes)?;
        let size = nodes.len();
        let mut kept = Vec::new();
        for (index, node) in nodes.into_iter().enumerate() {
            let context = Context {
                node,
                position: index + 1,
                size,
            };
            let holds = match self.eval(predicate, context)? {
                XValue::Number(value) => value == (index + 1) as f64,
                value => self.boolean_of(&value)?,
            };
            if holds {
                kept.push(node);
            }
        }
        Ok(kept)
    }

    /// The nodes of `axis` from `node`, in axis order.
    fn axis(&mut self, node: NodeId, axis: Axis) -> Result<Vec<NodeId>, EvalError> {
        let document = self.document;
        let current = document.node(node);
        let is_attribute = matches!(current.kind, NodeKind::Attribute(..));
        let nodes: Vec<NodeId> =
            match axis {
                Axis::Child => current.children.clone(),
                Axis::Attribute => current.attributes.clone(),
                Axis::Itself => vec![node],
                Axis::Parent => current.parent.into_iter().collect(),
                Axis::Descendant | Axis::DescendantOrSelf => {
                    let mut nodes = Vec::new();
                    if axis == Axis::DescendantOrSelf {
                        nodes.push(node);
                    }
                    if !is_attribute {
                        nodes.extend((node + 1..=current.subtree_end).filter(|id| {
                            !matches!(document.node(*id).kind, NodeKind::Attribute(..))
                        }));
                    }
                    nodes
                }
                Axis::Ancestor | Axis::AncestorOrSelf => {
                    let mut nodes = Vec::new();
                    if axis == Axis::AncestorOrSelf {
                        nodes.push(node);
                    }
                    let mut parent = current.parent;
                    while let Some(ancestor) = parent {
                        self.charge_one()?;
                        nodes.push(ancestor);
                        parent = document.node(ancestor).parent;
                    }
                    nodes
                }
                Axis::FollowingSibling | Axis::PrecedingSibling => {
                    let siblings = match (is_attribute, current.parent) {
                        (false, Some(parent)) => document.node(parent).children.as_slice(),
                        _ => &[],
                    };
                    let index = siblings.iter().position(|sibling| *sibling == node);
                    match (axis, index) {
                        (Axis::FollowingSibling, Some(index)) => siblings[index + 1..].to_vec(),
                        (_, Some(index)) => siblings[..index].iter().rev().copied().collect(),
                        _ => Vec::new(),
                    }
                }
                Axis::Following => {
                    let start = if is_attribute {
                        node + 1
                    } else {
                        current.subtree_end + 1
                    };
                    (start..document.len())
                        .filter(|id| !matches!(document.node(*id).kind, NodeKind::Attribute(..)))
                        .collect()
                }
                Axis::Preceding => (0..node)
                    .rev()
                    .filter(|id| {
                        let candidate = document.node(*id);
                        // An ancestor's subtree reaches the node.
                        candidate.subtree_end < node
                            && !matches!(candidate.kind, NodeKind::Attribute(..))
                    })
                    .collect(),
            };
        (self.charge)(nodes.len() as u64)?;
        Ok(nodes)
    }

    /// Refuses a node set holding a white-space text node, whose presence changes what is read
    /// from it.
    pub(super) fn observe(&self, nodes: &[NodeId]) -> Result<(), EvalError> {
        if nodes
            .iter()
            .any(|node| is_white_space_text(&self.document.node(*node).kind))
        {
            Err(EvalError::Unsupported)
        } else {
            Ok(())
        }
    }

    fn charge_one(&mut self) -> Result<(), EvalError> {
        (self.charge)(1)
    }

    /// The string value of a node (§5).
    pub(super) fn string_value(&mut self, node: NodeId) -> Result<String, EvalError> {
        let document = self.document;
        let current = document.node(node);
        let value = match &current.kind {
            NodeKind::Root | NodeKind::Element(_) => {
                let mut value = String::new();
                for id in node + 1..=current.subtree_end {
                    match &document.node(id).kind {
                        kind if is_white_space_text(kind) => return Err(EvalError::Unsupported),
                        NodeKind::Text(text) => value.push_str(text),
                        _ => {}
                    }
                }
                value
            }
            kind if is_white_space_text(kind) => return Err(EvalError::Unsupported),
            NodeKind::Attribute(_, value) | NodeKind::Text(value) | NodeKind::Comment(value) => {
                value.clone()
            }
            NodeKind::ProcessingInstruction { data, .. } => data.clone(),
        };
        (self.charge)(value.len() as u64 + 1)?;
        Ok(value)
    }

    fn boolean(&mut self, expr: &Expr, context: Context) -> Result<bool, EvalError> {
        let value = self.eval(expr, context)?;
        self.boolean_of(&value)
    }

    fn number(&mut self, expr: &Expr, context: Context) -> Result<f64, EvalError> {
        let value = self.eval(expr, context)?;
        self.number_of(&value)
    }

    fn string(&mut self, expr: &Expr, context: Context) -> Result<String, EvalError> {
        let value = self.eval(expr, context)?;
        self.string_of(&value)
    }

    /// Whether a value is true (§4.3); a node set of only white-space text nodes is unsupported.
    fn boolean_of(&self, value: &XValue) -> Result<bool, EvalError> {
        if let XValue::Nodes(nodes) = value
            && !nodes.is_empty()
            && nodes
                .iter()
                .all(|node| is_white_space_text(&self.document.node(*node).kind))
        {
            return Err(EvalError::Unsupported);
        }
        Ok(atom_boolean(value))
    }

    fn number_of(&mut self, value: &XValue) -> Result<f64, EvalError> {
        Ok(match value {
            XValue::Number(value) => *value,
            XValue::Boolean(value) => f64::from(u8::from(*value)),
            XValue::String(text) => string_number(text),
            XValue::Nodes(_) => string_number(&self.string_of(value)?),
        })
    }

    fn string_of(&mut self, value: &XValue) -> Result<String, EvalError> {
        Ok(match value {
            XValue::String(text) => text.clone(),
            XValue::Boolean(value) => value.to_string(),
            XValue::Number(value) => number_string(*value),
            XValue::Nodes(nodes) => match nodes.first() {
                Some(node) => self.string_value(*node)?,
                None => String::new(),
            },
        })
    }

    /// Compares two values (§3.4): a node set holds when some node's value does.
    fn compare(
        &mut self,
        comparison: Comparison,
        left: &XValue,
        right: &XValue,
    ) -> Result<bool, EvalError> {
        match (left, right) {
            (XValue::Nodes(left), XValue::Nodes(right)) => {
                let right = self.strings(right)?;
                for node in left {
                    let left = self.string_value(*node)?;
                    for right in &right {
                        if compare_atoms(
                            comparison,
                            &XValue::String(left.clone()),
                            &XValue::String(right.clone()),
                        ) {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
            (XValue::Nodes(nodes), XValue::Boolean(_)) => Ok(compare_atoms(
                comparison,
                &XValue::Boolean(!nodes.is_empty()),
                right,
            )),
            (XValue::Boolean(_), XValue::Nodes(nodes)) => Ok(compare_atoms(
                comparison,
                left,
                &XValue::Boolean(!nodes.is_empty()),
            )),
            (XValue::Nodes(nodes), atom) => {
                for node in nodes {
                    let text = XValue::String(self.string_value(*node)?);
                    let value = match atom {
                        XValue::Number(_) => XValue::Number(self.number_of(&text)?),
                        _ => text,
                    };
                    if compare_atoms(comparison, &value, atom) {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            (atom, XValue::Nodes(nodes)) => {
                for node in nodes {
                    let text = XValue::String(self.string_value(*node)?);
                    let value = match atom {
                        XValue::Number(_) => XValue::Number(self.number_of(&text)?),
                        _ => text,
                    };
                    if compare_atoms(comparison, atom, &value) {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(compare_atoms(comparison, left, right)),
        }
    }

    fn strings(&mut self, nodes: &[NodeId]) -> Result<Vec<String>, EvalError> {
        nodes.iter().map(|node| self.string_value(*node)).collect()
    }

    fn call(
        &mut self,
        function: Function,
        args: &[Expr],
        context: Context,
    ) -> Result<XValue, EvalError> {
        let node_argument = |evaluator: &mut Self| -> Result<Option<NodeId>, EvalError> {
            match args.first() {
                None => Ok(Some(context.node)),
                Some(expr) => match evaluator.eval(expr, context)? {
                    XValue::Nodes(nodes) => {
                        if let Some(first) = nodes.first() {
                            evaluator.observe(&[*first])?;
                        }
                        Ok(nodes.first().copied())
                    }
                    _ => Err(EvalError::Unsupported),
                },
            }
        };
        let string_argument = |evaluator: &mut Self, index: usize| match args.get(index) {
            Some(expr) => evaluator.string(expr, context),
            None => evaluator.string_value(context.node),
        };
        Ok(match function {
            Function::Last => XValue::Number(context.size as f64),
            Function::Position => XValue::Number(context.position as f64),
            Function::Count => match self.eval(&args[0], context)? {
                XValue::Nodes(nodes) => {
                    self.observe(&nodes)?;
                    XValue::Number(nodes.len() as f64)
                }
                _ => return Err(EvalError::Unsupported),
            },
            Function::LocalName | Function::NamespaceUri | Function::Name => {
                let name =
                    node_argument(self)?.and_then(|node| match &self.document.node(node).kind {
                        NodeKind::Element(name) | NodeKind::Attribute(name, _) => {
                            Some(match function {
                                Function::LocalName => name.local.clone(),
                                Function::NamespaceUri => {
                                    name.namespace.clone().unwrap_or_default()
                                }
                                _ => name.qualified(),
                            })
                        }
                        NodeKind::ProcessingInstruction { target, .. }
                            if function != Function::NamespaceUri =>
                        {
                            Some(target.clone())
                        }
                        _ => None,
                    });
                XValue::String(name.unwrap_or_default())
            }
            Function::String => XValue::String(string_argument(self, 0)?),
            Function::Concat => {
                let mut text = String::new();
                for arg in args {
                    text.push_str(&self.string(arg, context)?);
                }
                XValue::String(text)
            }
            Function::StartsWith | Function::Contains => {
                let text = self.string(&args[0], context)?;
                let pattern = self.string(&args[1], context)?;
                (self.charge)((text.len() + pattern.len()) as u64)?;
                XValue::Boolean(if function == Function::StartsWith {
                    text.starts_with(&pattern)
                } else {
                    text.contains(&pattern)
                })
            }
            Function::SubstringBefore | Function::SubstringAfter => {
                let text = self.string(&args[0], context)?;
                let pattern = self.string(&args[1], context)?;
                (self.charge)((text.len() + pattern.len()) as u64)?;
                XValue::String(match text.find(&pattern) {
                    Some(index) if function == Function::SubstringBefore => {
                        text[..index].to_owned()
                    }
                    Some(index) => text[index + pattern.len()..].to_owned(),
                    None => String::new(),
                })
            }
            Function::Substring => {
                let text = self.string(&args[0], context)?;
                let start = round(self.number(&args[1], context)?);
                let end = match args.get(2) {
                    Some(length) => start + round(self.number(length, context)?),
                    None => f64::INFINITY,
                };
                // §4.2: characters at positions p with start <= p < start + length, counted
                // from 1; NaN in either bound selects nothing.
                XValue::String(
                    text.chars()
                        .zip(1..)
                        .filter(|(_, position)| {
                            let position = f64::from(*position);
                            position >= start && position < end
                        })
                        .map(|(character, _)| character)
                        .collect(),
                )
            }
            Function::StringLength => {
                XValue::Number(string_argument(self, 0)?.chars().count() as f64)
            }
            Function::NormalizeSpace => XValue::String(
                string_argument(self, 0)?
                    .split(is_xml_space)
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            Function::Translate => {
                let text = self.string(&args[0], context)?;
                let from: Vec<char> = self.string(&args[1], context)?.chars().collect();
                let to: Vec<char> = self.string(&args[2], context)?.chars().collect();
                (self.charge)((text.len() * from.len().max(1)) as u64)?;
                XValue::String(
                    text.chars()
                        .filter_map(|character| {
                            match from.iter().position(|candidate| *candidate == character) {
                                Some(index) => to.get(index).copied(),
                                None => Some(character),
                            }
                        })
                        .collect(),
                )
            }
            Function::Boolean => XValue::Boolean(self.boolean(&args[0], context)?),
            Function::Not => XValue::Boolean(!self.boolean(&args[0], context)?),
            Function::True => XValue::Boolean(true),
            Function::False => XValue::Boolean(false),
            Function::Number => XValue::Number(match args.first() {
                Some(expr) => self.number(expr, context)?,
                None => string_number(&self.string_value(context.node)?),
            }),
            Function::Sum => match self.eval(&args[0], context)? {
                XValue::Nodes(nodes) => {
                    let mut total = 0.0;
                    for node in nodes {
                        total += string_number(&self.string_value(node)?);
                    }
                    XValue::Number(total)
                }
                _ => return Err(EvalError::Unsupported),
            },
            Function::Floor => XValue::Number(self.number(&args[0], context)?.floor()),
            Function::Ceiling => XValue::Number(self.number(&args[0], context)?.ceil()),
            Function::Round => XValue::Number(round(self.number(&args[0], context)?)),
        })
    }
}

/// Whether a value is true (§4.3).
fn atom_boolean(value: &XValue) -> bool {
    match value {
        XValue::Nodes(nodes) => !nodes.is_empty(),
        XValue::Boolean(value) => *value,
        XValue::Number(value) => *value != 0.0 && !value.is_nan(),
        XValue::String(text) => !text.is_empty(),
    }
}

/// Whether `node` passes `test` on `axis` (§2.3): a name test matches the axis's principal node
/// type, attributes on the attribute axis and elements elsewhere.
fn matches_test(document: &Document, node: NodeId, axis: Axis, test: &NodeTest) -> bool {
    let kind = &document.node(node).kind;
    let principal = match kind {
        NodeKind::Attribute(name, _) if axis == Axis::Attribute => Some(name),
        NodeKind::Element(name) if axis != Axis::Attribute => Some(name),
        _ => None,
    };
    match test {
        NodeTest::Any => principal.is_some(),
        NodeTest::Name(local) => {
            principal.is_some_and(|name| name.namespace.is_none() && name.local == *local)
        }
        NodeTest::Node => true,
        NodeTest::Text => matches!(kind, NodeKind::Text(_)),
        NodeTest::Comment => matches!(kind, NodeKind::Comment(_)),
        NodeTest::ProcessingInstruction(target) => matches!(
            kind,
            NodeKind::ProcessingInstruction { target: actual, .. }
                if target.as_ref().is_none_or(|target| target == actual)
        ),
    }
}

fn is_white_space_text(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Text(text) if text.chars().all(is_xml_space))
}

/// Compares two values that are not node sets (§3.4).
fn compare_atoms(comparison: Comparison, left: &XValue, right: &XValue) -> bool {
    let number = |value: &XValue| match value {
        XValue::Number(value) => *value,
        XValue::Boolean(value) => f64::from(u8::from(*value)),
        XValue::String(text) => string_number(text),
        XValue::Nodes(_) => f64::NAN,
    };
    match comparison {
        Comparison::Equal | Comparison::NotEqual => {
            let equal = match (left, right) {
                (XValue::Boolean(_), _) | (_, XValue::Boolean(_)) => {
                    atom_boolean(left) == atom_boolean(right)
                }
                (XValue::Number(_), _) | (_, XValue::Number(_)) => number(left) == number(right),
                (XValue::String(left), XValue::String(right)) => left == right,
                _ => false,
            };
            equal == (comparison == Comparison::Equal)
        }
        Comparison::Less => number(left) < number(right),
        Comparison::LessOrEqual => number(left) <= number(right),
        Comparison::Greater => number(left) > number(right),
        Comparison::GreaterOrEqual => number(left) >= number(right),
    }
}

/// `number()` of a string (§4.4): optional white space, an optional minus sign, and digits with
/// an optional decimal point; anything else is NaN.
fn string_number(text: &str) -> f64 {
    let trimmed = text.trim_matches(is_xml_space);
    let digits = trimmed.strip_prefix('-').unwrap_or(trimmed);
    let mut parts = digits.splitn(2, '.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    let valid = whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.is_none_or(|fraction| fraction.bytes().all(|byte| byte.is_ascii_digit()))
        && (!whole.is_empty() || fraction.is_some_and(|fraction| !fraction.is_empty()));
    if valid {
        trimmed.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// `string()` of a number (§4.2): no exponent, no trailing zeros, `NaN`, `Infinity`, and `0` for
/// negative zero.
fn number_string(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    }
}

/// `round()` (§4.4): the closest integer, the one toward positive infinity for a tie, keeping NaN,
/// infinities, and negative zero.
fn round(value: f64) -> f64 {
    if value.is_nan() || value.is_infinite() || value == 0.0 {
        return value;
    }
    let floor = value.floor();
    let rounded = if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    };
    if rounded == 0.0 && value < 0.0 {
        -0.0
    } else {
        rounded
    }
}
