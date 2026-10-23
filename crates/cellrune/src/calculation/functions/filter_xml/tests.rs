use super::document::{Document, DocumentError};
use super::evaluate::{EvalError, Evaluator, XValue};
use super::xpath::{ParseError, parse};

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Values(Vec<String>),
    Invalid,
    Unsupported,
}

fn select(xml: &str, path: &str) -> Outcome {
    let document = match Document::parse(xml) {
        Ok(document) => document,
        Err(DocumentError::Invalid) => return Outcome::Invalid,
        Err(DocumentError::Unsupported) => return Outcome::Unsupported,
    };
    let expr = match parse(path) {
        Ok(expr) => expr,
        Err(ParseError::Syntax | ParseError::Unsupported) => return Outcome::Unsupported,
    };
    let mut evaluator = Evaluator::new(&document, |_| Ok(()));
    let values = evaluator.evaluate(&expr).and_then(|value| match value {
        XValue::Nodes(nodes) => {
            evaluator.observe(&nodes)?;
            nodes
                .iter()
                .map(|node| evaluator.string_value(*node))
                .collect()
        }
        _ => Err(EvalError::Unsupported),
    });
    match values {
        Ok(values) => Outcome::Values(values),
        Err(_) => Outcome::Unsupported,
    }
}

fn values(items: &[&str]) -> Outcome {
    Outcome::Values(items.iter().map(|item| (*item).to_owned()).collect())
}

const TREE: &str = "<r><x k='1'>a</x><x k='2'>b</x><y><x k='3'>c</x><!--n--><?p d?></y></r>";

#[test]
fn location_paths_select_nodes_in_document_order() {
    for (path, expected) in [
        ("//x", &["a", "b", "c"][..]),
        ("/r/x", &["a", "b"]),
        ("//x[1]", &["a", "c"]),
        ("(//x)[1]", &["a"]),
        ("/r/x[last()]", &["b"]),
        ("//x[@k='2']", &["b"]),
        ("//@k", &["1", "2", "3"]),
        ("/r/*[position()>1]", &["b", "c"]),
        ("//x | /r/y/x | /r/x[1]", &["a", "b", "c"]),
        ("//y/preceding-sibling::x", &["a", "b"]),
        ("//y/preceding-sibling::x[1]", &["b"]),
        ("//x[3]/ancestor::*[1]", &[]),
        ("(//x)[3]/ancestor::*[1]", &["c"]),
        ("//x[. = 'a']/following::x", &["b", "c"]),
        ("//x[2]/preceding::*", &["a"]),
        ("//x[@k='3']/../..//x[not(@k>1)]", &["a"]),
        ("//comment()", &["n"]),
        ("//processing-instruction('p')", &["d"]),
        ("//text()", &["a", "b", "c"]),
        ("/r/x/@k/..", &["a", "b"]),
        ("//x[contains(., 'b') or starts-with(., 'c')]", &["b", "c"]),
        (
            "//x[string-length(concat(., @k)) = 2][@k mod 2 = 1]",
            &["a", "c"],
        ),
        ("//*[local-name() = 'x'][count(../x) = 2]", &["a", "b"]),
    ] {
        assert_eq!(select(TREE, path), values(expected), "{path}");
    }
}

#[test]
fn string_and_number_functions_follow_xpath_one() {
    // §4.2 and §4.4 examples, each turned into a predicate that keeps the document element.
    for predicate in [
        "substring('12345', 1.5, 2.6) = '234'",
        "substring('12345', 0, 3) = '12'",
        "substring('12345', 0 div 0, 3) = ''",
        "substring('12345', -42, 1 div 0) = '12345'",
        "substring('12345', -1 div 0, 1 div 0) = ''",
        "substring-before('1999/04/01', '/') = '1999'",
        "substring-after('1999/04/01', '19') = '99/04/01'",
        "translate('--aaa--', 'abc-', 'ABC') = 'AAA'",
        "normalize-space('  a  b ') = 'a b'",
        "string(1 div 0) = 'Infinity' and string(0 div 0) = 'NaN'",
        "string(-0) = '0' and string(2.50) = '2.5' and string(1000000) = '1000000'",
        "round(2.5) = 3 and round(-2.5) = -2 and round(0.49999999999999994) = 0",
        "floor(-1.5) = -2 and ceiling(1.2) = 2",
        "number(' 12 ') = 12 and string(number('1e3')) = 'NaN'",
        "sum(//@k) = 6 and 7 mod -3 = 1 and -7 mod 3 = -1",
        "boolean(//y) and not(//z) and true() and not(false())",
        "//x = 'b' and //x != 'a' and //@k > 2 and not(//@k > 3)",
        "//x[1] = //x[2] or //@k = 1",
        "name(/*) = 'r' and namespace-uri(/*) = ''",
    ] {
        assert_eq!(
            select(TREE, &format!("/r[{predicate}]")),
            values(&["abc"]),
            "{predicate}"
        );
    }
}

#[test]
fn documents_read_references_sections_and_namespaces() {
    assert_eq!(
        select("<r><x>a&amp;b&#x41;<![CDATA[<c>]]></x></r>", "//x"),
        values(&["a&bA<c>"])
    );
    assert_eq!(
        select("<?xml version='1.0'?><r a=' 1&lt;2 '/>", "/r/@a"),
        values(&[" 1<2 "])
    );
    // An unprefixed name test matches only elements in no namespace.
    let namespaced = "<r xmlns='urn:a' xmlns:p='urn:p'><x>a</x><p:x>b</p:x></r>";
    assert_eq!(select(namespaced, "//x"), values(&[]));
    assert_eq!(
        select(namespaced, "//*[local-name() = 'x']"),
        values(&["a", "b"])
    );
    assert_eq!(
        select(namespaced, "//*[name() = 'p:x']/namespace-uri()"),
        Outcome::Unsupported
    );
    assert_eq!(
        select(namespaced, "//*[namespace-uri() = 'urn:p']"),
        values(&["b"])
    );
}

#[test]
fn malformed_documents_are_invalid() {
    for xml in [
        "",
        "<r>",
        "<r></x>",
        "<r/><r/>",
        "text<r/>",
        "<p:r/>",
        "<r p:a='1'/>",
        "<r a='1' a='2'/>",
        "<r xmlns:p='u' xmlns:q='u' p:a='1' q:a='2'/>",
        "<r>&bogus;</r>",
        "<r>&#0;</r>",
        "<r a=1/>",
        "<1r/>",
        "<r><!-- a -- b --></r>",
    ] {
        assert_eq!(select(xml, "/"), Outcome::Invalid, "{xml}");
    }
    assert_eq!(
        select("<!DOCTYPE r [<!ENTITY e 'x'>]><r>&e;</r>", "/r"),
        Outcome::Unsupported
    );
}

#[test]
fn white_space_text_is_unsupported_only_where_it_could_be_read() {
    let pretty = "<r>\n  <x>a</x>\n  <x>b</x>\n</r>";
    assert_eq!(select(pretty, "//x"), values(&["a", "b"]));
    assert_eq!(select(pretty, "/r/x[2]"), values(&["b"]));
    assert_eq!(select(pretty, "//x[. = 'b']"), values(&["b"]));
    for path in [
        "/r",
        "//text()",
        "/r/node()[2]",
        "count(/r/node())",
        "//text()/..",
        "/r[. = 'ab']",
    ] {
        assert_eq!(select(pretty, path), Outcome::Unsupported, "{path}");
    }
}

#[test]
fn expressions_outside_the_modeled_language_are_unsupported() {
    for path in [
        "$v",
        "//p:x",
        "//p:*",
        "namespace::*",
        "id('a')",
        "lang('en')",
        "unknown()",
        "//x[",
        "//",
        "count()",
        "'text'",
        "1 + 1",
        "//x = 'a'",
    ] {
        assert_eq!(select(TREE, path), Outcome::Unsupported, "{path}");
    }
}
