//! XPath 1.0 expressions (W3C Recommendation, 16 November 1999): lexical structure (§3.7) and
//! grammar.
//!
//! Variable references, namespace prefixes in name tests, the `namespace` axis, and the `id` and
//! `lang` functions need context Excel does not document, so they parse as unsupported.

use super::document::{is_name, is_name_character, is_xml_space};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseError {
    /// The text is not an XPath 1.0 expression.
    Syntax,
    /// The expression uses a construct this evaluator does not model.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Expr {
    Or(Box<Self>, Box<Self>),
    And(Box<Self>, Box<Self>),
    Compare(Comparison, Box<Self>, Box<Self>),
    Arithmetic(Arithmetic, Box<Self>, Box<Self>),
    Negate(Box<Self>),
    Union(Box<Self>, Box<Self>),
    Path(LocationPath),
    /// A primary expression with predicates, followed by relative steps.
    Filter {
        primary: Box<Self>,
        predicates: Vec<Self>,
        steps: Vec<Step>,
    },
    Literal(String),
    Number(f64),
    Call(Function, Vec<Self>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Arithmetic {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LocationPath {
    pub(super) absolute: bool,
    pub(super) steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Step {
    pub(super) axis: Axis,
    pub(super) test: NodeTest,
    pub(super) predicates: Vec<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Axis {
    Ancestor,
    AncestorOrSelf,
    Attribute,
    Child,
    Descendant,
    DescendantOrSelf,
    Following,
    FollowingSibling,
    Parent,
    Preceding,
    PrecedingSibling,
    Itself,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NodeTest {
    /// `*`: every node of the axis's principal node type.
    Any,
    /// An unprefixed name: nodes of the principal type with that local name and no namespace.
    Name(String),
    Node,
    Text,
    Comment,
    ProcessingInstruction(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Function {
    Last,
    Position,
    Count,
    LocalName,
    NamespaceUri,
    Name,
    String,
    Concat,
    StartsWith,
    Contains,
    SubstringBefore,
    SubstringAfter,
    Substring,
    StringLength,
    NormalizeSpace,
    Translate,
    Boolean,
    Not,
    True,
    False,
    Number,
    Sum,
    Floor,
    Ceiling,
    Round,
}

impl Function {
    fn from_name(name: &str) -> Result<Self, ParseError> {
        Ok(match name {
            "last" => Self::Last,
            "position" => Self::Position,
            "count" => Self::Count,
            "local-name" => Self::LocalName,
            "namespace-uri" => Self::NamespaceUri,
            "name" => Self::Name,
            "string" => Self::String,
            "concat" => Self::Concat,
            "starts-with" => Self::StartsWith,
            "contains" => Self::Contains,
            "substring-before" => Self::SubstringBefore,
            "substring-after" => Self::SubstringAfter,
            "substring" => Self::Substring,
            "string-length" => Self::StringLength,
            "normalize-space" => Self::NormalizeSpace,
            "translate" => Self::Translate,
            "boolean" => Self::Boolean,
            "not" => Self::Not,
            "true" => Self::True,
            "false" => Self::False,
            "number" => Self::Number,
            "sum" => Self::Sum,
            "floor" => Self::Floor,
            "ceiling" => Self::Ceiling,
            "round" => Self::Round,
            _ => return Err(ParseError::Unsupported),
        })
    }

    /// Whether `count` arguments fit the function's signature (§4).
    const fn accepts(self, count: usize) -> bool {
        match self {
            Self::Last | Self::Position | Self::True | Self::False => count == 0,
            Self::Count
            | Self::Boolean
            | Self::Not
            | Self::Sum
            | Self::Floor
            | Self::Ceiling
            | Self::Round => count == 1,
            Self::LocalName
            | Self::NamespaceUri
            | Self::Name
            | Self::String
            | Self::StringLength
            | Self::NormalizeSpace
            | Self::Number => count <= 1,
            Self::StartsWith | Self::Contains | Self::SubstringBefore | Self::SubstringAfter => {
                count == 2
            }
            Self::Substring => count == 2 || count == 3,
            Self::Translate => count == 3,
            Self::Concat => count >= 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Dot,
    DotDot,
    At,
    Comma,
    ColonColon,
    Slash,
    SlashSlash,
    Pipe,
    Plus,
    Minus,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    /// `*` read as multiplication.
    Multiply,
    /// `and`, `or`, `mod`, or `div` read as an operator.
    OperatorName(String),
    /// `*` read as a name test.
    Star,
    Name(String),
    Literal(String),
    Number(f64),
}

impl Token {
    /// Whether a following `*` or name is an operator (§3.7): the token is not `@`, `::`, `(`,
    /// `[`, `,`, or an operator.
    const fn ends_operand(&self) -> bool {
        !matches!(
            self,
            Self::At
                | Self::ColonColon
                | Self::LeftParen
                | Self::LeftBracket
                | Self::Comma
                | Self::Slash
                | Self::SlashSlash
                | Self::Pipe
                | Self::Plus
                | Self::Minus
                | Self::Equal
                | Self::NotEqual
                | Self::Less
                | Self::LessOrEqual
                | Self::Greater
                | Self::GreaterOrEqual
                | Self::Multiply
                | Self::OperatorName(_)
        )
    }
}

fn tokenize(text: &str) -> Result<Vec<Token>, ParseError> {
    let characters: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if is_xml_space(character) {
            index += 1;
            continue;
        }
        let next = characters.get(index + 1).copied();
        let operand_before = tokens.last().is_some_and(Token::ends_operand);
        let (token, length) = match character {
            '(' => (Token::LeftParen, 1),
            ')' => (Token::RightParen, 1),
            '[' => (Token::LeftBracket, 1),
            ']' => (Token::RightBracket, 1),
            '@' => (Token::At, 1),
            ',' => (Token::Comma, 1),
            '|' => (Token::Pipe, 1),
            '+' => (Token::Plus, 1),
            '-' => (Token::Minus, 1),
            '=' => (Token::Equal, 1),
            '!' if next == Some('=') => (Token::NotEqual, 2),
            '<' if next == Some('=') => (Token::LessOrEqual, 2),
            '<' => (Token::Less, 1),
            '>' if next == Some('=') => (Token::GreaterOrEqual, 2),
            '>' => (Token::Greater, 1),
            '/' if next == Some('/') => (Token::SlashSlash, 2),
            '/' => (Token::Slash, 1),
            ':' if next == Some(':') => (Token::ColonColon, 2),
            '*' if operand_before => (Token::Multiply, 1),
            '*' => (Token::Star, 1),
            '$' => return Err(ParseError::Unsupported),
            '"' | '\'' => {
                let end = characters[index + 1..]
                    .iter()
                    .position(|candidate| *candidate == character)
                    .ok_or(ParseError::Syntax)?;
                let literal = characters[index + 1..index + 1 + end].iter().collect();
                (Token::Literal(literal), end + 2)
            }
            '.' if next == Some('.') => (Token::DotDot, 2),
            '.' if !next.is_some_and(|digit| digit.is_ascii_digit()) => (Token::Dot, 1),
            '.' | '0'..='9' => {
                let mut end = index;
                while characters.get(end).is_some_and(char::is_ascii_digit) {
                    end += 1;
                }
                if characters.get(end) == Some(&'.') {
                    end += 1;
                    while characters.get(end).is_some_and(char::is_ascii_digit) {
                        end += 1;
                    }
                }
                let number: String = characters[index..end].iter().collect();
                let value = number.parse::<f64>().map_err(|_| ParseError::Syntax)?;
                (Token::Number(value), end - index)
            }
            _ => {
                let mut end = index;
                while characters.get(end).copied().is_some_and(is_name_character) {
                    end += 1;
                }
                let name: String = characters[index..end].iter().collect();
                if !is_name(&name) {
                    return Err(ParseError::Syntax);
                }
                // A prefix (`p:name` or `p:*`) needs namespace bindings Excel does not document.
                if characters.get(end) == Some(&':') && characters.get(end + 1) != Some(&':') {
                    return Err(ParseError::Unsupported);
                }
                let token =
                    if operand_before && matches!(name.as_str(), "and" | "or" | "mod" | "div") {
                        Token::OperatorName(name)
                    } else {
                        Token::Name(name)
                    };
                (token, end - index)
            }
        };
        tokens.push(token);
        index += length;
    }
    Ok(tokens)
}

/// Parses an XPath 1.0 expression.
pub(super) fn parse(text: &str) -> Result<Expr, ParseError> {
    let tokens = tokenize(text)?;
    let mut parser = Parser { tokens, cursor: 0 };
    let expr = parser.or_expr()?;
    if parser.cursor != parser.tokens.len() {
        return Err(ParseError::Syntax);
    }
    Ok(expr)
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.cursor + offset)
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Token) -> Result<(), ParseError> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(ParseError::Syntax)
        }
    }

    fn eat_operator_name(&mut self, name: &str) -> bool {
        if matches!(self.peek(), Some(Token::OperatorName(operator)) if operator == name) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn or_expr(&mut self) -> Result<Expr, ParseError> {
        crate::calculation::stack::grow(|| {
            let mut left = self.and_expr()?;
            while self.eat_operator_name("or") {
                left = Expr::Or(Box::new(left), Box::new(self.and_expr()?));
            }
            Ok(left)
        })
    }

    fn and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.equality_expr()?;
        while self.eat_operator_name("and") {
            left = Expr::And(Box::new(left), Box::new(self.equality_expr()?));
        }
        Ok(left)
    }

    fn equality_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.relational_expr()?;
        loop {
            let comparison = match self.peek() {
                Some(Token::Equal) => Comparison::Equal,
                Some(Token::NotEqual) => Comparison::NotEqual,
                _ => return Ok(left),
            };
            self.cursor += 1;
            left = Expr::Compare(
                comparison,
                Box::new(left),
                Box::new(self.relational_expr()?),
            );
        }
    }

    fn relational_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.additive_expr()?;
        loop {
            let comparison = match self.peek() {
                Some(Token::Less) => Comparison::Less,
                Some(Token::LessOrEqual) => Comparison::LessOrEqual,
                Some(Token::Greater) => Comparison::Greater,
                Some(Token::GreaterOrEqual) => Comparison::GreaterOrEqual,
                _ => return Ok(left),
            };
            self.cursor += 1;
            left = Expr::Compare(comparison, Box::new(left), Box::new(self.additive_expr()?));
        }
    }

    fn additive_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.multiplicative_expr()?;
        loop {
            let operation = match self.peek() {
                Some(Token::Plus) => Arithmetic::Add,
                Some(Token::Minus) => Arithmetic::Subtract,
                _ => return Ok(left),
            };
            self.cursor += 1;
            left = Expr::Arithmetic(
                operation,
                Box::new(left),
                Box::new(self.multiplicative_expr()?),
            );
        }
    }

    fn multiplicative_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.unary_expr()?;
        loop {
            let operation = match self.peek() {
                Some(Token::Multiply) => Arithmetic::Multiply,
                Some(Token::OperatorName(name)) if name == "div" => Arithmetic::Divide,
                Some(Token::OperatorName(name)) if name == "mod" => Arithmetic::Modulo,
                _ => return Ok(left),
            };
            self.cursor += 1;
            left = Expr::Arithmetic(operation, Box::new(left), Box::new(self.unary_expr()?));
        }
    }

    fn unary_expr(&mut self) -> Result<Expr, ParseError> {
        if self.eat(&Token::Minus) {
            return crate::calculation::stack::grow(|| {
                Ok(Expr::Negate(Box::new(self.unary_expr()?)))
            });
        }
        let mut left = self.path_expr()?;
        while self.eat(&Token::Pipe) {
            left = Expr::Union(Box::new(left), Box::new(self.path_expr()?));
        }
        Ok(left)
    }

    fn path_expr(&mut self) -> Result<Expr, ParseError> {
        if self.starts_filter_expr() {
            let primary = self.primary_expr()?;
            let predicates = self.predicates()?;
            let steps = if matches!(self.peek(), Some(Token::Slash | Token::SlashSlash)) {
                self.relative_steps(Vec::new(), true)?
            } else {
                Vec::new()
            };
            if predicates.is_empty() && steps.is_empty() {
                return Ok(primary);
            }
            return Ok(Expr::Filter {
                primary: Box::new(primary),
                predicates,
                steps,
            });
        }
        let absolute = matches!(self.peek(), Some(Token::Slash | Token::SlashSlash));
        let mut steps = Vec::new();
        if absolute {
            let descendants = self.peek() == Some(&Token::SlashSlash);
            self.cursor += 1;
            if descendants {
                steps.push(descendant_or_self_step());
            } else if !self.starts_step() {
                return Ok(Expr::Path(LocationPath {
                    absolute: true,
                    steps,
                }));
            }
        }
        let steps = self.relative_steps(steps, false)?;
        Ok(Expr::Path(LocationPath { absolute, steps }))
    }

    /// Whether the next tokens start a primary expression rather than a location path: a literal,
    /// number, parenthesis, or function name that is not a node type (§3.7).
    fn starts_filter_expr(&self) -> bool {
        match self.peek() {
            Some(Token::Literal(_) | Token::Number(_) | Token::LeftParen) => true,
            Some(Token::Name(name)) => {
                self.peek_at(1) == Some(&Token::LeftParen) && !is_node_type(name)
            }
            _ => false,
        }
    }

    fn starts_step(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token::Dot | Token::DotDot | Token::At | Token::Star | Token::Name(_))
        )
    }

    /// Reads `step (('/' | '//') step)*`; with `leading_separator`, a separator comes first.
    fn relative_steps(
        &mut self,
        mut steps: Vec<Step>,
        leading_separator: bool,
    ) -> Result<Vec<Step>, ParseError> {
        if leading_separator {
            if self.eat(&Token::SlashSlash) {
                steps.push(descendant_or_self_step());
            } else {
                self.expect(&Token::Slash)?;
            }
        }
        loop {
            steps.push(self.step()?);
            if self.eat(&Token::SlashSlash) {
                steps.push(descendant_or_self_step());
            } else if !self.eat(&Token::Slash) {
                return Ok(steps);
            }
        }
    }

    fn step(&mut self) -> Result<Step, ParseError> {
        if self.eat(&Token::Dot) {
            return Ok(Step {
                axis: Axis::Itself,
                test: NodeTest::Node,
                predicates: Vec::new(),
            });
        }
        if self.eat(&Token::DotDot) {
            return Ok(Step {
                axis: Axis::Parent,
                test: NodeTest::Node,
                predicates: Vec::new(),
            });
        }
        let axis = if self.eat(&Token::At) {
            Axis::Attribute
        } else if let (Some(Token::Name(name)), Some(Token::ColonColon)) =
            (self.peek(), self.peek_at(1))
        {
            let axis = axis_from_name(name)?;
            self.cursor += 2;
            axis
        } else {
            Axis::Child
        };
        let test = self.node_test()?;
        let predicates = self.predicates()?;
        Ok(Step {
            axis,
            test,
            predicates,
        })
    }

    fn node_test(&mut self) -> Result<NodeTest, ParseError> {
        match self.peek().cloned() {
            Some(Token::Star) => {
                self.cursor += 1;
                Ok(NodeTest::Any)
            }
            Some(Token::Name(name)) if self.peek_at(1) == Some(&Token::LeftParen) => {
                if !is_node_type(&name) {
                    return Err(ParseError::Syntax);
                }
                self.cursor += 2;
                let test = match name.as_str() {
                    "node" => NodeTest::Node,
                    "text" => NodeTest::Text,
                    "comment" => NodeTest::Comment,
                    _ => match self.peek().cloned() {
                        Some(Token::Literal(target)) => {
                            self.cursor += 1;
                            NodeTest::ProcessingInstruction(Some(target))
                        }
                        _ => NodeTest::ProcessingInstruction(None),
                    },
                };
                self.expect(&Token::RightParen)?;
                Ok(test)
            }
            Some(Token::Name(name)) => {
                self.cursor += 1;
                Ok(NodeTest::Name(name))
            }
            _ => Err(ParseError::Syntax),
        }
    }

    fn predicates(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut predicates = Vec::new();
        while self.eat(&Token::LeftBracket) {
            predicates.push(self.or_expr()?);
            self.expect(&Token::RightBracket)?;
        }
        Ok(predicates)
    }

    fn primary_expr(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            Some(Token::Literal(text)) => {
                self.cursor += 1;
                Ok(Expr::Literal(text))
            }
            Some(Token::Number(value)) => {
                self.cursor += 1;
                Ok(Expr::Number(value))
            }
            Some(Token::LeftParen) => {
                self.cursor += 1;
                let expr = self.or_expr()?;
                self.expect(&Token::RightParen)?;
                Ok(expr)
            }
            Some(Token::Name(name)) => {
                self.cursor += 2;
                let function = match name.as_str() {
                    "id" | "lang" => return Err(ParseError::Unsupported),
                    _ => Function::from_name(&name)?,
                };
                let mut args = Vec::new();
                if !self.eat(&Token::RightParen) {
                    loop {
                        args.push(self.or_expr()?);
                        if self.eat(&Token::RightParen) {
                            break;
                        }
                        self.expect(&Token::Comma)?;
                    }
                }
                if !function.accepts(args.len()) {
                    return Err(ParseError::Syntax);
                }
                Ok(Expr::Call(function, args))
            }
            _ => Err(ParseError::Syntax),
        }
    }
}

fn is_node_type(name: &str) -> bool {
    matches!(name, "comment" | "text" | "processing-instruction" | "node")
}

fn axis_from_name(name: &str) -> Result<Axis, ParseError> {
    Ok(match name {
        "ancestor" => Axis::Ancestor,
        "ancestor-or-self" => Axis::AncestorOrSelf,
        "attribute" => Axis::Attribute,
        "child" => Axis::Child,
        "descendant" => Axis::Descendant,
        "descendant-or-self" => Axis::DescendantOrSelf,
        "following" => Axis::Following,
        "following-sibling" => Axis::FollowingSibling,
        "parent" => Axis::Parent,
        "preceding" => Axis::Preceding,
        "preceding-sibling" => Axis::PrecedingSibling,
        "self" => Axis::Itself,
        "namespace" => return Err(ParseError::Unsupported),
        _ => return Err(ParseError::Syntax),
    })
}

/// `//` abbreviates `/descendant-or-self::node()/`.
const fn descendant_or_self_step() -> Step {
    Step {
        axis: Axis::DescendantOrSelf,
        test: NodeTest::Node,
        predicates: Vec::new(),
    }
}
