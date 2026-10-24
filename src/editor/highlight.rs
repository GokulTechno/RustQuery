//! A small, dependency-free SQL tokenizer used for editor highlighting.
//!
//! Tokens are stored per line as character ranges. Multi-line comments,
//! strings and PostgreSQL dollar-quoted blocks are supported by carrying a
//! lexer state from one line to the next.

use std::collections::HashSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Function,
    Type,
    String,
    QuotedIdentifier,
    Number,
    Comment,
    Operator,
    Variable,
    Punctuation,
    Identifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

#[derive(Debug, Default)]
pub struct SyntaxCache {
    pub lines: Vec<Vec<Token>>,
    dirty: bool,
}

impl SyntaxCache {
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn ensure(&mut self, source_lines: &[String]) {
        if !self.dirty && self.lines.len() == source_lines.len() {
            return;
        }
        self.lines = highlight_lines(source_lines);
        self.dirty = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LexState {
    Normal,
    BlockComment,
    SingleQuote,
    DoubleQuote,
    Backtick,
    /// PostgreSQL dollar quote including the delimiter, e.g. `$body$`.
    DollarQuote(String),
}

/// Re-tokenize the full buffer. SQL files are small enough for this to be
/// imperceptible; caching avoids the cost on every rendered frame.
pub fn highlight_lines(lines: &[String]) -> Vec<Vec<Token>> {
    let mut state = LexState::Normal;
    lines
        .iter()
        .map(|line| {
            let (tokens, next) = tokenize_line(line, &state);
            state = next;
            tokens
        })
        .collect()
}

fn tokenize_line(line: &str, initial: &LexState) -> (Vec<Token>, LexState) {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let mut state = initial.clone();

    // Continue a construct that started on a previous line.
    match state.clone() {
        LexState::BlockComment => {
            let end = find_block_comment_end(&chars, 0);
            match end {
                Some(end_idx) => {
                    push(&mut tokens, 0, end_idx, TokenKind::Comment);
                    i = end_idx;
                    state = LexState::Normal;
                }
                None => {
                    if n > 0 {
                        push(&mut tokens, 0, n, TokenKind::Comment);
                    }
                    return (tokens, LexState::BlockComment);
                }
            }
        }
        LexState::SingleQuote => {
            let (end, closed) = find_quoted_end(&chars, 0, '\'');
            push(&mut tokens, 0, end, TokenKind::String);
            if !closed {
                return (tokens, LexState::SingleQuote);
            }
            i = end;
            state = LexState::Normal;
        }
        LexState::DoubleQuote => {
            let (end, closed) = find_quoted_end(&chars, 0, '"');
            push(&mut tokens, 0, end, TokenKind::QuotedIdentifier);
            if !closed {
                return (tokens, LexState::DoubleQuote);
            }
            i = end;
            state = LexState::Normal;
        }
        LexState::Backtick => {
            let (end, closed) = find_quoted_end(&chars, 0, '`');
            push(&mut tokens, 0, end, TokenKind::QuotedIdentifier);
            if !closed {
                return (tokens, LexState::Backtick);
            }
            i = end;
            state = LexState::Normal;
        }
        LexState::DollarQuote(tag) => match find_tag(line, &tag, 0) {
            Some(end_idx) => {
                push_chars(&mut tokens, 0, end_idx, TokenKind::String);
                i = end_idx;
                state = LexState::Normal;
            }
            None => {
                if n > 0 {
                    push(&mut tokens, 0, n, TokenKind::String);
                }
                return (tokens, LexState::DollarQuote(tag));
            }
        },
        LexState::Normal => {}
    }

    while i < n {
        let c = chars[i];

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // Line comment.
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            push(&mut tokens, i, n, TokenKind::Comment);
            break;
        }

        // Block comment.
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            if let Some(end) = find_block_comment_end(&chars, i + 2) {
                push(&mut tokens, i, end, TokenKind::Comment);
                i = end;
            } else {
                push(&mut tokens, i, n, TokenKind::Comment);
                state = LexState::BlockComment;
                break;
            }
            continue;
        }

        if matches!(c, '\'' | '"' | '`') {
            let quote = c;
            let (end, closed) = find_quoted_end(&chars, i + 1, quote);
            let kind = if quote == '\'' {
                TokenKind::String
            } else {
                TokenKind::QuotedIdentifier
            };
            push(&mut tokens, i, end, kind);
            i = end;
            if !closed {
                state = match quote {
                    '\'' => LexState::SingleQuote,
                    '"' => LexState::DoubleQuote,
                    _ => LexState::Backtick,
                };
                break;
            }
            continue;
        }

        // Dollar quote or positional parameter.
        if c == '$' {
            if let Some((tag, after_tag)) = dollar_tag(&chars, i) {
                let rest: String = chars[after_tag..].iter().collect();
                if let Some(offset) = rest.find(&tag) {
                    let end = after_tag + rest[..offset].chars().count() + tag.chars().count();
                    push(&mut tokens, i, end, TokenKind::String);
                    i = end;
                } else {
                    push(&mut tokens, i, n, TokenKind::String);
                    state = LexState::DollarQuote(tag);
                    break;
                }
                continue;
            }
            let end = consume_word(&chars, i);
            let kind = if chars
                .get(i + 1)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                TokenKind::Variable
            } else {
                TokenKind::Operator
            };
            push(&mut tokens, i, end, kind);
            i = end;
            continue;
        }

        if c.is_ascii_digit() {
            let end = consume_number(&chars, i);
            push(&mut tokens, i, end, TokenKind::Number);
            i = end;
            continue;
        }

        if is_word_start(c) {
            let end = consume_word(&chars, i);
            let word: String = chars[i..end].iter().collect();
            let upper = word.to_ascii_uppercase();
            let keyword = is_keyword(&upper);
            let ty = is_type(&upper);
            let mut kind = if keyword {
                TokenKind::Keyword
            } else if ty {
                TokenKind::Type
            } else {
                TokenKind::Identifier
            };
            if !keyword && !ty && looks_like_call(&chars, end) {
                kind = TokenKind::Function;
            }
            push(&mut tokens, i, end, kind);
            i = end;
            continue;
        }

        if is_operator(c) {
            let start = i;
            i += 1;
            while i < n && is_operator(chars[i]) {
                // Keep comment openers out of operator runs.
                if (chars[i] == '-' && chars.get(i + 1) == Some(&'-'))
                    || (chars[i] == '/' && chars.get(i + 1) == Some(&'*'))
                {
                    break;
                }
                i += 1;
            }
            push(&mut tokens, start, i, TokenKind::Operator);
            continue;
        }

        push(&mut tokens, i, i + 1, TokenKind::Punctuation);
        i += 1;
    }

    (tokens, state)
}

fn push(tokens: &mut Vec<Token>, start: usize, end: usize, kind: TokenKind) {
    if end <= start {
        return;
    }
    if let Some(last) = tokens.last_mut() {
        if last.kind == kind && last.end == start {
            last.end = end;
            return;
        }
    }
    tokens.push(Token { start, end, kind });
}

fn push_chars(tokens: &mut Vec<Token>, start: usize, end: usize, kind: TokenKind) {
    push(tokens, start, end, kind);
}

fn find_block_comment_end(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < chars.len() {
        if chars[i] == '*' && chars[i + 1] == '/' {
            return Some(i + 2);
        }
        i += 1;
    }
    None
}

/// Find the end of a quoted region, honouring doubled quote escapes.
/// Returns (index_after_closing_quote_or_end_of_line, closed).
fn find_quoted_end(chars: &[char], from: usize, quote: char) -> (usize, bool) {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == quote {
            if chars.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return (i + 1, true);
        }
        i += 1;
    }
    (chars.len(), false)
}

fn dollar_tag(chars: &[char], at: usize) -> Option<(String, usize)> {
    let mut i = at + 1;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' {
            let tag: String = chars[at..=i].iter().collect();
            return Some((tag, i + 1));
        }
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        // Guard against scanning absurdly far for a tag.
        if i - at > 64 {
            return None;
        }
        i += 1;
    }
    None
}

fn find_tag(line: &str, tag: &str, from: usize) -> Option<usize> {
    line.get(from..).and_then(|rest| rest.find(tag)).map(|idx| {
        let prefix_chars = line[..from + idx].chars().count();
        prefix_chars + tag.chars().count()
    })
}

fn consume_word(chars: &[char], from: usize) -> usize {
    let mut i = from;
    while i < chars.len() {
        let c = chars[i];
        if c.is_alphanumeric() || c == '_' || c == '$' {
            i += 1;
        } else {
            break;
        }
    }
    i
}

fn consume_number(chars: &[char], from: usize) -> usize {
    let mut i = from;
    let mut seen_dot = false;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            i += 1;
        } else if c == '.' && !seen_dot {
            // Do not swallow a qualified name or range operator.
            if chars
                .get(i + 1)
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                seen_dot = true;
                i += 1;
            } else {
                break;
            }
        } else if (c == 'e' || c == 'E')
            && i + 1 < chars.len()
            && (chars[i + 1].is_ascii_digit()
                || (matches!(chars[i + 1], '+' | '-')
                    && chars
                        .get(i + 2)
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false)))
        {
            i += 2;
        } else {
            break;
        }
    }
    i
}

fn looks_like_call(chars: &[char], from: usize) -> bool {
    let mut i = from;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    chars.get(i) == Some(&'(')
}

fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '+' | '-'
            | '*'
            | '/'
            | '%'
            | '='
            | '<'
            | '>'
            | '!'
            | '|'
            | '&'
            | '^'
            | '~'
            | ':'
            | '@'
            | '?'
    )
}

fn keywords() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        [
            "ADD",
            "ALL",
            "ALTER",
            "ANALYZE",
            "AND",
            "ANY",
            "AS",
            "ASC",
            "ASYMMETRIC",
            "AT",
            "AUTHORIZATION",
            "AUTO_INCREMENT",
            "BEGIN",
            "BETWEEN",
            "BIGINT",
            "BINARY",
            "BOTH",
            "BY",
            "CASE",
            "CASCADE",
            "CAST",
            "CHANGE",
            "CHARACTER",
            "CHECK",
            "COLLATE",
            "COLUMN",
            "COMMENT",
            "COMMIT",
            "CONCURRENTLY",
            "CONFLICT",
            "CONSTRAINT",
            "CREATE",
            "CROSS",
            "CURRENT",
            "CURRENT_DATE",
            "CURRENT_TIME",
            "CURRENT_TIMESTAMP",
            "CURRENT_USER",
            "DATABASE",
            "DATABASES",
            "DEFAULT",
            "DEFERRABLE",
            "DELETE",
            "DESC",
            "DESCRIBE",
            "DISTINCT",
            "DO",
            "DROP",
            "ELSE",
            "END",
            "ESCAPE",
            "EXCEPT",
            "EXCLUDE",
            "EXISTS",
            "EXPLAIN",
            "FALSE",
            "FETCH",
            "FILTER",
            "FIRST",
            "FOLLOWING",
            "FOR",
            "FOREIGN",
            "FROM",
            "FULL",
            "FUNCTION",
            "GENERATED",
            "GRANT",
            "GROUP",
            "HAVING",
            "IF",
            "ILIKE",
            "IN",
            "INDEX",
            "INNER",
            "INSERT",
            "INTERSECT",
            "INTO",
            "IS",
            "ISNULL",
            "JOIN",
            "KEY",
            "LATERAL",
            "LEADING",
            "LEFT",
            "LIKE",
            "LIMIT",
            "LOCK",
            "MATERIALIZED",
            "NATURAL",
            "NOT",
            "NOTNULL",
            "NULL",
            "NULLS",
            "OFFSET",
            "ON",
            "ONLY",
            "OR",
            "ORDER",
            "OUTER",
            "OVER",
            "PARTITION",
            "PRECEDING",
            "PRIMARY",
            "PROCEDURE",
            "RANGE",
            "RECURSIVE",
            "REFERENCES",
            "REFRESH",
            "REINDEX",
            "RENAME",
            "REPLACE",
            "RESTRICT",
            "RETURNING",
            "REVOKE",
            "RIGHT",
            "ROLLBACK",
            "ROW",
            "ROWS",
            "SAVEPOINT",
            "SCHEMA",
            "SELECT",
            "SEQUENCE",
            "SESSION",
            "SET",
            "SHOW",
            "SOME",
            "START",
            "SYMMETRIC",
            "TABLE",
            "TABLES",
            "TABLESAMPLE",
            "TEMP",
            "TEMPORARY",
            "THEN",
            "TIES",
            "TO",
            "TRAILING",
            "TRANSACTION",
            "TRIGGER",
            "TRUE",
            "TRUNCATE",
            "UNBOUNDED",
            "UNION",
            "UNIQUE",
            "UNLOGGED",
            "UPDATE",
            "USE",
            "USING",
            "VACUUM",
            "VALUES",
            "VIEW",
            "WHEN",
            "WHERE",
            "WINDOW",
            "WITH",
            "WITHOUT",
            "ZONE",
        ]
        .into_iter()
        .collect()
    })
}

fn types() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        [
            "ARRAY",
            "BIGSERIAL",
            "BIGINT",
            "BIT",
            "BLOB",
            "BOOL",
            "BOOLEAN",
            "BPCHAR",
            "BYTEA",
            "CHAR",
            "CIDR",
            "DATE",
            "DATETIME",
            "DECIMAL",
            "DOUBLE",
            "ENUM",
            "FLOAT",
            "FLOAT4",
            "FLOAT8",
            "INET",
            "INT",
            "INT2",
            "INT4",
            "INT8",
            "INTEGER",
            "INTERVAL",
            "JSON",
            "JSONB",
            "LONGTEXT",
            "MACADDR",
            "MEDIUMINT",
            "MEDIUMTEXT",
            "MONEY",
            "NUMERIC",
            "REAL",
            "SERIAL",
            "SET",
            "SMALLINT",
            "SMALLSERIAL",
            "TEXT",
            "TIME",
            "TIMESTAMP",
            "TIMESTAMPTZ",
            "TIMETZ",
            "TINYINT",
            "TINYTEXT",
            "UUID",
            "VARBINARY",
            "VARCHAR",
            "VARCHAR2",
            "XML",
            "YEAR",
        ]
        .into_iter()
        .collect()
    })
}

pub fn is_keyword(upper: &str) -> bool {
    keywords().contains(upper)
}

pub fn is_type(upper: &str) -> bool {
    types().contains(upper)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<TokenKind> {
        tokenize_line(line, &LexState::Normal)
            .0
            .iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn highlights_keywords_and_functions() {
        let ks = kinds("SELECT count(*) FROM users");
        assert_eq!(ks[0], TokenKind::Keyword);
        assert_eq!(ks[1], TokenKind::Function);
        assert!(ks.contains(&TokenKind::Keyword));
    }

    #[test]
    fn strings_and_comments() {
        let ks = kinds("SELECT '-- not a comment' -- comment");
        assert_eq!(ks.iter().filter(|k| **k == TokenKind::String).count(), 1);
        assert_eq!(ks.last(), Some(&TokenKind::Comment));
    }

    #[test]
    fn multiline_block_comments_carry_state() {
        let lines = vec![
            "SELECT 1 /* start".to_string(),
            "still comment".to_string(),
            "end */ SELECT 2".to_string(),
        ];
        let out = highlight_lines(&lines);
        assert_eq!(out[1][0].kind, TokenKind::Comment);
        assert!(out[2].iter().any(|t| t.kind == TokenKind::Keyword));
    }

    #[test]
    fn dollar_quotes() {
        let lines = vec!["SELECT $tag$ hello".to_string(), "world $tag$;".to_string()];
        let out = highlight_lines(&lines);
        assert_eq!(out[0].last().unwrap().kind, TokenKind::String);
        assert_eq!(out[1][0].kind, TokenKind::String);
    }

    #[test]
    fn numbers() {
        let ks = kinds("SELECT 12.5, 1e10, 42");
        assert_eq!(ks.iter().filter(|k| **k == TokenKind::Number).count(), 3);
    }
}
