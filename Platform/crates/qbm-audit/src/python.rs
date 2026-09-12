//! Bounded, non-executing Python source scanner.
//!
//! This intentionally extracts a conservative structural subset instead of
//! importing Python or evaluating its AST. The immutable source-size ceiling in
//! `GraphPolicy` and the token ceiling below bound scanner memory and work.

use std::collections::{BTreeMap, BTreeSet};

/// Maximum lexical tokens retained for one Python artifact.
const PYTHON_TOKEN_LIMIT: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PythonScan {
    pub(super) module: String,
    pub(super) declarations: Vec<PythonDeclaration>,
    pub(super) imports: Vec<PythonImport>,
    pub(super) calls: Vec<PythonCall>,
    pub(super) diagnostics: Vec<PythonDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PythonDeclarationKind {
    Class,
    Function,
    TestClass,
    TestFunction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PythonDeclaration {
    pub(super) kind: PythonDeclarationKind,
    pub(super) name: String,
    pub(super) qualified_name: String,
    pub(super) parent: Option<usize>,
    pub(super) line: u64,
    pub(super) column: u64,
    pub(super) is_async: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PythonImport {
    pub(super) owner: Option<usize>,
    pub(super) reference: String,
    pub(super) binding: Option<String>,
    pub(super) line: u64,
    pub(super) column: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PythonCall {
    pub(super) owner: Option<usize>,
    pub(super) reference: String,
    pub(super) expanded_reference: String,
    pub(super) line: u64,
    pub(super) column: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PythonDiagnostic {
    pub(super) code: &'static str,
    pub(super) message: String,
    pub(super) line: u64,
    pub(super) column: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogicalLine {
    indent: usize,
    tokens: Vec<Token>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: TokenKind,
    line: u64,
    column: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Name(String),
    Dot,
    Comma,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Colon,
    Star,
    Semicolon,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Delimiter {
    symbol: char,
    line: u64,
    column: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TripleString {
    quote: char,
    line: u64,
    column: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Scope {
    indent: usize,
    declaration: usize,
}

pub(super) fn scan(source: &str, relative_path: &str) -> PythonScan {
    let module = python_module_name(relative_path);
    let (lines, mut diagnostics) = tokenize(source);
    let mut declarations = Vec::new();
    let mut imports = Vec::new();
    let mut calls = Vec::new();
    let mut scopes = Vec::<Scope>::new();
    let mut bindings = BTreeMap::<String, String>::new();

    for line in &lines {
        while scopes
            .last()
            .is_some_and(|scope| scope.indent >= line.indent)
        {
            scopes.pop();
        }
        let parent = scopes.last().map(|scope| scope.declaration);
        let declaration = parse_declaration(line, &module, parent, &declarations, &mut diagnostics);
        let call_owner = if let Some(mut declaration) = declaration {
            let index = declarations.len();
            declaration.parent = parent;
            declarations.push(declaration);
            scopes.push(Scope {
                indent: line.indent,
                declaration: index,
            });
            Some(index)
        } else {
            parent
        };

        let owner = declaration_header_boundary(&line.tokens).map_or(parent, |_| call_owner);
        for import in parse_imports(line, parent, &module, &mut diagnostics) {
            if let Some(binding) = &import.binding {
                bindings.insert(binding.clone(), import.reference.clone());
            }
            imports.push(import);
        }
        calls.extend(parse_calls(line, owner, &bindings));
    }

    imports.sort_by(|left, right| {
        (left.line, left.column, &left.reference, &left.binding).cmp(&(
            right.line,
            right.column,
            &right.reference,
            &right.binding,
        ))
    });
    imports.dedup();
    calls.sort_by(|left, right| {
        (
            left.line,
            left.column,
            &left.reference,
            &left.expanded_reference,
        )
            .cmp(&(
                right.line,
                right.column,
                &right.reference,
                &right.expanded_reference,
            ))
    });
    calls.dedup();
    diagnostics.sort_by(|left, right| {
        (left.line, left.column, left.code, &left.message).cmp(&(
            right.line,
            right.column,
            right.code,
            &right.message,
        ))
    });
    diagnostics.dedup();

    PythonScan {
        module,
        declarations,
        imports,
        calls,
        diagnostics,
    }
}

#[allow(clippy::too_many_lines)] // Keeping lexical state transitions together makes bounds auditable.
fn tokenize(source: &str) -> (Vec<LogicalLine>, Vec<PythonDiagnostic>) {
    let mut logical_lines = Vec::new();
    let mut diagnostics = Vec::new();
    let mut current_tokens = Vec::new();
    let mut current_indent = 0;
    let mut delimiters = Vec::<Delimiter>::new();
    let mut triple_string = None::<TripleString>;
    let mut explicit_continuation = false;
    let mut indentation_stack = vec![0_usize];
    let mut previous_requires_suite = false;
    let mut token_limit_reached = false;

    for (line_index, physical_line) in source.split_inclusive('\n').enumerate() {
        let line_number = u64::try_from(line_index).unwrap_or(u64::MAX) + 1;
        let text = physical_line
            .strip_suffix('\n')
            .unwrap_or(physical_line)
            .strip_suffix('\r')
            .unwrap_or_else(|| physical_line.strip_suffix('\n').unwrap_or(physical_line));
        let chars = text.chars().collect::<Vec<_>>();
        let starts_logical_line = current_tokens.is_empty()
            && delimiters.is_empty()
            && triple_string.is_none()
            && !explicit_continuation;
        let (indent, content_start, mixed_indent) = indentation(&chars);
        if starts_logical_line {
            current_indent = indent;
        }
        if mixed_indent {
            diagnostics.push(PythonDiagnostic {
                code: "python_indentation_error",
                message: "Python indentation prefix mixes tabs and spaces".to_owned(),
                line: line_number,
                column: 1,
            });
        }

        let mut index = if starts_logical_line {
            content_start
        } else {
            0
        };
        let blank_or_comment = chars.get(index).is_none_or(|character| *character == '#');
        if starts_logical_line && !blank_or_comment {
            validate_indentation(
                indent,
                previous_requires_suite,
                &mut indentation_stack,
                line_number,
                &mut diagnostics,
            );
        }
        let tokens_before = current_tokens.len();
        explicit_continuation = false;

        while index < chars.len() {
            if token_limit_reached {
                break;
            }
            if let Some(state) = triple_string {
                if triple_quote_at(&chars, index, state.quote) {
                    triple_string = None;
                    index += 3;
                } else {
                    index += 1;
                }
                continue;
            }
            let character = chars[index];
            if character.is_whitespace() {
                index += 1;
                continue;
            }
            if character == '#' {
                break;
            }
            if character == '\\' && index + 1 == chars.len() {
                explicit_continuation = true;
                index += 1;
                continue;
            }
            if character == '\'' || character == '"' {
                let column = u64::try_from(index).unwrap_or(u64::MAX) + 1;
                if triple_quote_at(&chars, index, character) {
                    triple_string = Some(TripleString {
                        quote: character,
                        line: line_number,
                        column,
                    });
                    index += 3;
                    continue;
                }
                if let Some(next) = skip_short_string(&chars, index, character) {
                    index = next;
                } else {
                    diagnostics.push(PythonDiagnostic {
                        code: "python_syntax_error",
                        message: "Unterminated Python string literal".to_owned(),
                        line: line_number,
                        column,
                    });
                    index = chars.len();
                }
                continue;
            }

            let column = u64::try_from(index).unwrap_or(u64::MAX) + 1;
            let (kind, next) = if is_identifier_start(character) {
                let mut end = index + 1;
                while end < chars.len() && is_identifier_continue(chars[end]) {
                    end += 1;
                }
                (TokenKind::Name(chars[index..end].iter().collect()), end)
            } else {
                let kind = match character {
                    '.' => TokenKind::Dot,
                    ',' => TokenKind::Comma,
                    '(' => TokenKind::LeftParen,
                    ')' => TokenKind::RightParen,
                    '[' => TokenKind::LeftBracket,
                    ']' => TokenKind::RightBracket,
                    '{' => TokenKind::LeftBrace,
                    '}' => TokenKind::RightBrace,
                    ':' => TokenKind::Colon,
                    '*' => TokenKind::Star,
                    ';' => TokenKind::Semicolon,
                    _ => TokenKind::Other,
                };
                (kind, index + 1)
            };
            update_delimiters(
                &kind,
                line_number,
                column,
                &mut delimiters,
                &mut diagnostics,
            );
            current_tokens.push(Token {
                kind,
                line: line_number,
                column,
            });
            if current_tokens.len() >= PYTHON_TOKEN_LIMIT {
                diagnostics.push(PythonDiagnostic {
                    code: "python_token_limit",
                    message: format!(
                        "Python lexical token limit of {PYTHON_TOKEN_LIMIT} was reached; structural scanning stopped"
                    ),
                    line: line_number,
                    column,
                });
                token_limit_reached = true;
            }
            index = next;
        }

        let added_tokens = current_tokens.len() > tokens_before;
        if delimiters.is_empty() && triple_string.is_none() && !explicit_continuation {
            if !current_tokens.is_empty() {
                previous_requires_suite = matches!(
                    current_tokens.last().map(|token| &token.kind),
                    Some(TokenKind::Colon)
                );
                logical_lines.push(LogicalLine {
                    indent: current_indent,
                    tokens: std::mem::take(&mut current_tokens),
                });
            } else if added_tokens {
                previous_requires_suite = false;
            }
        }
        if token_limit_reached {
            break;
        }
    }

    if let Some(state) = triple_string {
        diagnostics.push(PythonDiagnostic {
            code: "python_syntax_error",
            message: "Unterminated Python triple-quoted string literal".to_owned(),
            line: state.line,
            column: state.column,
        });
    }
    for delimiter in delimiters {
        diagnostics.push(PythonDiagnostic {
            code: "python_syntax_error",
            message: format!("Unclosed Python delimiter {:?}", delimiter.symbol),
            line: delimiter.line,
            column: delimiter.column,
        });
    }
    if !current_tokens.is_empty() && !token_limit_reached {
        logical_lines.push(LogicalLine {
            indent: current_indent,
            tokens: current_tokens,
        });
    }
    (logical_lines, diagnostics)
}

fn indentation(chars: &[char]) -> (usize, usize, bool) {
    let mut width = 0_usize;
    let mut index = 0_usize;
    let mut has_space = false;
    let mut has_tab = false;
    while let Some(character) = chars.get(index) {
        match character {
            ' ' => {
                has_space = true;
                width = width.saturating_add(1);
            }
            '\t' => {
                has_tab = true;
                width = (width / 8 + 1).saturating_mul(8);
            }
            '\u{000c}' => width = 0,
            _ => break,
        }
        index += 1;
    }
    (width, index, has_space && has_tab)
}

fn validate_indentation(
    indent: usize,
    previous_requires_suite: bool,
    stack: &mut Vec<usize>,
    line: u64,
    diagnostics: &mut Vec<PythonDiagnostic>,
) {
    let current = *stack.last().unwrap_or(&0);
    if indent > current {
        if !previous_requires_suite {
            diagnostics.push(PythonDiagnostic {
                code: "python_indentation_error",
                message: "Unexpected Python indentation increase".to_owned(),
                line,
                column: 1,
            });
        }
        stack.push(indent);
    } else if indent < current {
        while stack.last().is_some_and(|value| *value > indent) {
            stack.pop();
        }
        if stack.last().copied() != Some(indent) {
            diagnostics.push(PythonDiagnostic {
                code: "python_indentation_error",
                message: "Python dedent does not match an earlier indentation level".to_owned(),
                line,
                column: 1,
            });
            stack.push(indent);
        }
    } else if previous_requires_suite {
        diagnostics.push(PythonDiagnostic {
            code: "python_indentation_error",
            message: "Python suite is not indented after a colon".to_owned(),
            line,
            column: 1,
        });
    }
}

fn update_delimiters(
    kind: &TokenKind,
    line: u64,
    column: u64,
    stack: &mut Vec<Delimiter>,
    diagnostics: &mut Vec<PythonDiagnostic>,
) {
    let opening = match kind {
        TokenKind::LeftParen => Some('('),
        TokenKind::LeftBracket => Some('['),
        TokenKind::LeftBrace => Some('{'),
        _ => None,
    };
    if let Some(symbol) = opening {
        stack.push(Delimiter {
            symbol,
            line,
            column,
        });
        return;
    }
    let expected = match kind {
        TokenKind::RightParen => Some('('),
        TokenKind::RightBracket => Some('['),
        TokenKind::RightBrace => Some('{'),
        _ => None,
    };
    if let Some(expected) = expected {
        if stack.last().is_some_and(|value| value.symbol == expected) {
            stack.pop();
        } else {
            diagnostics.push(PythonDiagnostic {
                code: "python_syntax_error",
                message: "Mismatched or unexpected Python closing delimiter".to_owned(),
                line,
                column,
            });
        }
    }
}

fn parse_declaration(
    line: &LogicalLine,
    module: &str,
    parent: Option<usize>,
    declarations: &[PythonDeclaration],
    diagnostics: &mut Vec<PythonDiagnostic>,
) -> Option<PythonDeclaration> {
    let tokens = statement_tokens(&line.tokens);
    let (keyword_index, is_async) = match (name_at(tokens, 0), name_at(tokens, 1)) {
        (Some("async"), Some("def")) => (1, true),
        (Some("def" | "class"), _) => (0, false),
        _ => return None,
    };
    let keyword = name_at(tokens, keyword_index).unwrap_or_default();
    let location = &tokens[keyword_index];
    let Some(name) = name_at(tokens, keyword_index + 1) else {
        diagnostics.push(PythonDiagnostic {
            code: "python_syntax_error",
            message: format!("Python {keyword} declaration has no valid name"),
            line: location.line,
            column: location.column,
        });
        return None;
    };
    let has_colon = tokens
        .iter()
        .skip(keyword_index + 2)
        .any(|token| token.kind == TokenKind::Colon);
    let function_has_parameters = keyword != "def"
        || matches!(
            tokens.get(keyword_index + 2).map(|token| &token.kind),
            Some(TokenKind::LeftParen)
        );
    if !has_colon || !function_has_parameters {
        diagnostics.push(PythonDiagnostic {
            code: "python_syntax_error",
            message: format!("Malformed Python {keyword} declaration for {name:?}"),
            line: location.line,
            column: location.column,
        });
        return None;
    }
    let qualified_name = parent.map_or_else(
        || format!("{module}.{name}"),
        |index| format!("{}.{}", declarations[index].qualified_name, name),
    );
    let parent_is_test = parent.is_some_and(|index| {
        matches!(
            declarations[index].kind,
            PythonDeclarationKind::TestClass | PythonDeclarationKind::TestFunction
        )
    });
    let kind = match keyword {
        "class" if name.starts_with("Test") || parent_is_test => PythonDeclarationKind::TestClass,
        "class" => PythonDeclarationKind::Class,
        "def" if name.starts_with("test_") || parent_is_test => PythonDeclarationKind::TestFunction,
        "def" => PythonDeclarationKind::Function,
        _ => return None,
    };
    Some(PythonDeclaration {
        kind,
        name: name.to_owned(),
        qualified_name,
        parent,
        line: tokens[keyword_index + 1].line,
        column: tokens[keyword_index + 1].column,
        is_async,
    })
}

fn parse_imports(
    line: &LogicalLine,
    owner: Option<usize>,
    module: &str,
    diagnostics: &mut Vec<PythonDiagnostic>,
) -> Vec<PythonImport> {
    let mut output = Vec::new();
    for statement in split_statements(&line.tokens) {
        match name_at(statement, 0) {
            Some("import") => parse_plain_import(statement, owner, &mut output, diagnostics),
            Some("from") => parse_from_import(statement, owner, module, &mut output, diagnostics),
            _ => {}
        }
    }
    output
}

fn parse_plain_import(
    tokens: &[Token],
    owner: Option<usize>,
    output: &mut Vec<PythonImport>,
    diagnostics: &mut Vec<PythonDiagnostic>,
) {
    let mut index = 1;
    let mut observed = false;
    while index < tokens.len() {
        let start = index;
        let Some((reference, next)) = dotted_name(tokens, index) else {
            diagnostics.push(import_diagnostic(
                tokens,
                "Malformed Python import statement",
            ));
            return;
        };
        index = next;
        let mut binding = reference.split('.').next().map(str::to_owned);
        if name_at(tokens, index) == Some("as") {
            let Some(alias) = name_at(tokens, index + 1) else {
                diagnostics.push(import_diagnostic(tokens, "Python import alias has no name"));
                return;
            };
            binding = Some(alias.to_owned());
            index += 2;
        }
        output.push(PythonImport {
            owner,
            reference,
            binding,
            line: tokens[start].line,
            column: tokens[start].column,
        });
        observed = true;
        if matches!(
            tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Comma)
        ) {
            index += 1;
        } else if index < tokens.len() {
            diagnostics.push(import_diagnostic(tokens, "Malformed Python import list"));
            return;
        }
    }
    if !observed {
        diagnostics.push(import_diagnostic(
            tokens,
            "Python import has no module name",
        ));
    }
}

#[allow(clippy::too_many_lines)] // Import grammar is clearer as one bounded state machine.
fn parse_from_import(
    tokens: &[Token],
    owner: Option<usize>,
    module: &str,
    output: &mut Vec<PythonImport>,
    diagnostics: &mut Vec<PythonDiagnostic>,
) {
    let mut index = 1;
    let mut relative_level = 0_usize;
    while matches!(
        tokens.get(index).map(|token| &token.kind),
        Some(TokenKind::Dot)
    ) {
        relative_level += 1;
        index += 1;
    }
    let (base, next) = dotted_name(tokens, index).unwrap_or_else(|| (String::new(), index));
    index = next;
    if name_at(tokens, index) != Some("import") {
        diagnostics.push(import_diagnostic(
            tokens,
            "Python from-import has no import keyword",
        ));
        return;
    }
    index += 1;
    if matches!(
        tokens.get(index).map(|token| &token.kind),
        Some(TokenKind::LeftParen)
    ) {
        index += 1;
    }
    let absolute_base = absolute_import_base(module, relative_level, &base);
    let mut observed = false;
    while index < tokens.len() {
        if matches!(
            tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::RightParen)
        ) {
            break;
        }
        let start = index;
        let (name, next) = if matches!(
            tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Star)
        ) {
            ("*".to_owned(), index + 1)
        } else if let Some(name) = name_at(tokens, index) {
            (name.to_owned(), index + 1)
        } else {
            diagnostics.push(import_diagnostic(
                tokens,
                "Malformed Python from-import list",
            ));
            return;
        };
        index = next;
        let reference = if name == "*" || absolute_base.is_empty() {
            if name == "*" {
                absolute_base.clone()
            } else {
                name.clone()
            }
        } else {
            format!("{absolute_base}.{name}")
        };
        let mut binding = (name != "*").then(|| name.clone());
        if name_at(tokens, index) == Some("as") {
            let Some(alias) = name_at(tokens, index + 1) else {
                diagnostics.push(import_diagnostic(
                    tokens,
                    "Python from-import alias has no name",
                ));
                return;
            };
            binding = Some(alias.to_owned());
            index += 2;
        }
        if !reference.is_empty() {
            output.push(PythonImport {
                owner,
                reference,
                binding,
                line: tokens[start].line,
                column: tokens[start].column,
            });
        }
        observed = true;
        if matches!(
            tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Comma)
        ) {
            index += 1;
        } else if matches!(
            tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::RightParen)
        ) {
            break;
        } else if index < tokens.len() {
            diagnostics.push(import_diagnostic(
                tokens,
                "Malformed Python from-import list",
            ));
            return;
        }
    }
    if !observed {
        diagnostics.push(import_diagnostic(
            tokens,
            "Python from-import has no imported name",
        ));
    }
}

fn parse_calls(
    line: &LogicalLine,
    owner: Option<usize>,
    bindings: &BTreeMap<String, String>,
) -> Vec<PythonCall> {
    let mut calls = Vec::new();
    let declaration_boundary = declaration_header_boundary(&line.tokens);
    let mut observed = BTreeSet::new();
    for (index, token) in line.tokens.iter().enumerate() {
        if token.kind != TokenKind::LeftParen || index == 0 {
            continue;
        }
        let Some((start, reference)) = dotted_name_backwards(&line.tokens, index - 1) else {
            continue;
        };
        let last = reference.rsplit('.').next().unwrap_or_default();
        if python_non_call_keyword(last)
            || declaration_boundary.is_some_and(|boundary| start <= boundary)
            || name_at(&line.tokens, start.saturating_sub(1))
                .is_some_and(|name| matches!(name, "def" | "class" | "import" | "from"))
        {
            continue;
        }
        let expanded_reference = expand_binding(&reference, bindings);
        let location = &line.tokens[start];
        if observed.insert((location.line, location.column, reference.clone())) {
            calls.push(PythonCall {
                owner,
                reference,
                expanded_reference,
                line: location.line,
                column: location.column,
            });
        }
    }
    calls
}

fn declaration_header_boundary(tokens: &[Token]) -> Option<usize> {
    let start = usize::from(name_at(tokens, 0) == Some("async"));
    if !matches!(name_at(tokens, start), Some("def" | "class")) {
        return None;
    }
    tokens
        .iter()
        .enumerate()
        .skip(start + 1)
        .filter(|(_, token)| token.kind == TokenKind::Colon)
        .map(|(index, _)| index)
        .next_back()
}

fn split_statements(tokens: &[Token]) -> Vec<&[Token]> {
    let mut statements = Vec::new();
    let mut start = 0;
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LeftParen | TokenKind::LeftBracket | TokenKind::LeftBrace => depth += 1,
            TokenKind::RightParen | TokenKind::RightBracket | TokenKind::RightBrace => {
                depth = depth.saturating_sub(1);
            }
            TokenKind::Semicolon if depth == 0 => {
                if start < index {
                    statements.push(&tokens[start..index]);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < tokens.len() {
        statements.push(&tokens[start..]);
    }
    statements
}

fn statement_tokens(tokens: &[Token]) -> &[Token] {
    split_statements(tokens)
        .into_iter()
        .next()
        .unwrap_or(tokens)
}

fn dotted_name(tokens: &[Token], start: usize) -> Option<(String, usize)> {
    let first = name_at(tokens, start)?;
    let mut names = vec![first];
    let mut index = start + 1;
    while matches!(
        tokens.get(index).map(|token| &token.kind),
        Some(TokenKind::Dot)
    ) {
        let name = name_at(tokens, index + 1)?;
        names.push(name);
        index += 2;
    }
    Some((names.join("."), index))
}

fn dotted_name_backwards(tokens: &[Token], end: usize) -> Option<(usize, String)> {
    let mut index = end;
    let mut names = vec![name_at(tokens, index)?];
    while index >= 2 && tokens[index - 1].kind == TokenKind::Dot {
        let name = name_at(tokens, index - 2)?;
        names.push(name);
        index -= 2;
    }
    names.reverse();
    Some((index, names.join(".")))
}

fn name_at(tokens: &[Token], index: usize) -> Option<&str> {
    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::Name(name)) => Some(name),
        _ => None,
    }
}

fn absolute_import_base(module: &str, relative_level: usize, base: &str) -> String {
    if relative_level == 0 {
        return base.to_owned();
    }
    let mut parts = module.split('.').collect::<Vec<_>>();
    if !parts.is_empty() {
        parts.pop();
    }
    for _ in 1..relative_level {
        parts.pop();
    }
    if !base.is_empty() {
        parts.extend(base.split('.'));
    }
    parts.join(".")
}

fn expand_binding(reference: &str, bindings: &BTreeMap<String, String>) -> String {
    let (head, tail) = reference
        .split_once('.')
        .map_or((reference, None), |(head, tail)| (head, Some(tail)));
    let Some(expanded) = bindings.get(head) else {
        return reference.to_owned();
    };
    tail.map_or_else(|| expanded.clone(), |tail| format!("{expanded}.{tail}"))
}

fn python_module_name(relative_path: &str) -> String {
    let without_extension = relative_path
        .strip_suffix(".py")
        .or_else(|| relative_path.strip_suffix(".PY"))
        .unwrap_or(relative_path);
    let mut parts = without_extension
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.last().copied() == Some("__init__") {
        parts.pop();
    }
    if parts.is_empty() {
        "__init__".to_owned()
    } else {
        parts.join(".")
    }
}

fn import_diagnostic(tokens: &[Token], message: &str) -> PythonDiagnostic {
    let location = tokens.first();
    PythonDiagnostic {
        code: "python_syntax_error",
        message: message.to_owned(),
        line: location.map_or(1, |token| token.line),
        column: location.map_or(1, |token| token.column),
    }
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn triple_quote_at(chars: &[char], index: usize, quote: char) -> bool {
    chars.get(index..index.saturating_add(3)) == Some(&[quote, quote, quote])
}

fn skip_short_string(chars: &[char], start: usize, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (offset, character) in chars.iter().enumerate().skip(start + 1) {
        if escaped {
            escaped = false;
        } else if *character == '\\' {
            escaped = true;
        } else if *character == quote {
            return Some(offset + 1);
        }
    }
    None
}

fn python_non_call_keyword(name: &str) -> bool {
    matches!(
        name,
        "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "case"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "match"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_nested_declarations_imports_calls_and_tests() {
        let source = r"
import numpy as np
from .helpers import normalize as clean

class Pipeline:
    def run(self, values):
        def finish(item):
            return clean(item)
        return np.asarray(finish(values))

class TestPipeline:
    async def test_run(self):
        assert Pipeline().run([])
";
        let scan = scan(source, "pkg/engine.py");
        assert!(scan.diagnostics.is_empty(), "{:?}", scan.diagnostics);
        assert_eq!(scan.module, "pkg.engine");
        assert!(
            scan.declarations
                .iter()
                .any(|item| item.qualified_name == "pkg.engine.Pipeline.run.finish")
        );
        assert!(scan.declarations.iter().any(|item| {
            item.qualified_name == "pkg.engine.TestPipeline.test_run"
                && item.kind == PythonDeclarationKind::TestFunction
                && item.is_async
        }));
        assert!(
            scan.imports
                .iter()
                .any(|item| item.reference == "numpy" && item.binding.as_deref() == Some("np"))
        );
        assert!(scan.imports.iter().any(|item| {
            item.reference == "pkg.helpers.normalize" && item.binding.as_deref() == Some("clean")
        }));
        assert!(
            scan.calls
                .iter()
                .any(|item| item.expanded_reference == "numpy.asarray")
        );
        assert!(
            scan.calls
                .iter()
                .any(|item| item.expanded_reference == "pkg.helpers.normalize")
        );
    }

    #[test]
    fn reports_syntax_and_indentation_errors_without_panicking() {
        let scan = scan("def broken(:\n  call(]\n", "broken.py");
        assert!(scan.diagnostics.iter().any(|item| {
            matches!(
                item.code,
                "python_syntax_error" | "python_indentation_error"
            )
        }));
    }

    #[test]
    fn output_is_deterministic() {
        let source = "from pkg import tool\ndef run():\n    return tool()\n";
        assert_eq!(scan(source, "app.py"), scan(source, "app.py"));
    }

    #[test]
    fn skips_strings_comments_and_control_parentheses() {
        let source = r#"
# fake()
text = "also_fake()"
if (ready):
    real()
"#;
        let scan = scan(source, "app.py");
        assert_eq!(
            scan.calls
                .iter()
                .map(|call| call.reference.as_str())
                .collect::<Vec<_>>(),
            vec!["real"]
        );
    }

    #[test]
    fn token_limit_bounds_adversarial_input() {
        let source = "x ".repeat(PYTHON_TOKEN_LIMIT + 10);
        let scan = scan(&source, "large.py");
        assert!(
            scan.diagnostics
                .iter()
                .any(|item| item.code == "python_token_limit")
        );
    }
}
